//! Ordered, bounded handoff from proxy observations to durable recording.

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TransportSequence(pub(crate) u64);

/// A bounded handoff from the synchronous metadata sink to the transport dispatcher.
#[derive(Debug)]
pub(crate) enum TransportDispatchJob {
    Metadata {
        metadata: ForwardMetadata,
        latch: Arc<TransportLatch>,
    },
    Response {
        forward: ForwardId,
        observation: Box<ResponseObservation>,
        predecessor: Arc<TransportLatch>,
    },
    #[cfg(test)]
    Placeholder {
        sequence: TransportSequence,
        latch: Arc<TransportLatch>,
    },
}

#[derive(Debug)]
pub(super) struct TransportResponseAdmissionFailure {
    observation: Box<ResponseObservation>,
    count: bool,
}

/// Coordinates bounded transport admission with its terminal provider observation.
///
/// Metadata is placed on the dispatcher queue before its latch becomes visible. A response that
/// observes the latch is therefore queued behind the metadata on the same FIFO dispatcher, while
/// an overflow response is explicitly degraded instead of waiting for an unbounded task.
#[derive(Debug)]
pub(crate) struct TransportOrdering {
    sender: tokio::sync::mpsc::Sender<TransportDispatchJob>,
    pending: Mutex<BTreeMap<TransportSequence, Arc<TransportLatch>>>,
}

#[derive(Debug, Default)]
pub(crate) struct TransportLatch {
    complete: Mutex<bool>,
    wake: Condvar,
}
impl TransportOrdering {
    pub(crate) const fn new(sender: tokio::sync::mpsc::Sender<TransportDispatchJob>) -> Self {
        Self {
            sender,
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    /// Tries to reserve both the bounded correlation slot and its queue slot.
    ///
    /// The latch is allocated only after both bounds admit the forward. The metadata job is sent
    /// before the latch is inserted, so a response cannot overtake it on the FIFO dispatcher.
    fn try_enqueue_transport(&self, metadata: ForwardMetadata) -> bool {
        let sequence = TransportSequence(metadata.forward.get());
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.contains_key(&sequence) || pending.len() >= IN_FLIGHT_FORWARDS {
            return false;
        }
        let Ok(permit) = self.sender.try_reserve() else {
            return false;
        };
        let latch = Arc::new(TransportLatch::default());
        permit.send(TransportDispatchJob::Metadata {
            metadata,
            latch: Arc::clone(&latch),
        });
        let _previous = pending.insert(sequence, latch);
        true
    }

    #[cfg(test)]
    pub(crate) fn try_enqueue_placeholder(&self, sequence: TransportSequence) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.contains_key(&sequence) || pending.len() >= IN_FLIGHT_FORWARDS {
            return false;
        }
        let Ok(permit) = self.sender.try_reserve() else {
            return false;
        };
        let latch = Arc::new(TransportLatch::default());
        permit.send(TransportDispatchJob::Placeholder {
            sequence,
            latch: Arc::clone(&latch),
        });
        let _previous = pending.insert(sequence, latch);
        true
    }

    /// Queues a response behind accepted metadata, or returns it for degraded direct recording.
    ///
    /// Removing the ticket only happens after the dispatcher slot is reserved. If the queue is
    /// saturated, the metadata ticket is removed and the caller records the response as degraded
    /// without waiting.
    fn try_enqueue_response(
        &self,
        forward: ForwardId,
        observation: Box<ResponseObservation>,
    ) -> Result<(), TransportResponseAdmissionFailure> {
        let sequence = TransportSequence(forward.get());
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(predecessor) = pending.get(&sequence).cloned() else {
            drop(pending);
            return Err(TransportResponseAdmissionFailure {
                observation,
                count: false,
            });
        };
        let Ok(permit) = self.sender.try_reserve() else {
            let _removed = pending.remove(&sequence);
            drop(pending);
            return Err(TransportResponseAdmissionFailure {
                observation,
                count: true,
            });
        };
        let _removed = pending.remove(&sequence);
        drop(pending);
        permit.send(TransportDispatchJob::Response {
            forward,
            observation,
            predecessor,
        });
        Ok(())
    }

    fn release_transport(&self, forward: ForwardId) {
        let sequence = TransportSequence(forward.get());
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _removed = pending.remove(&sequence);
    }

    /// Returns the number of accepted transport tickets not yet paired with a response.
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl TransportLatch {
    fn complete(&self) {
        let mut complete = self
            .complete
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *complete = true;
        drop(complete);
        self.wake.notify_all();
    }

    fn wait(&self) {
        let mut complete = self
            .complete
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*complete {
            complete = self
                .wake
                .wait(complete)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        drop(complete);
    }
}

/// Runs the sole fixed transport dispatcher worker.
pub(super) async fn run_transport_dispatcher(
    mut jobs: tokio::sync::mpsc::Receiver<TransportDispatchJob>,
    sender: tokio::sync::mpsc::Sender<RunEvent>,
) {
    let mut recorder_open = true;
    while let Some(job) = jobs.recv().await {
        match job {
            TransportDispatchJob::Metadata { metadata, latch } => {
                if recorder_open && sender.send(RunEvent::Transport(metadata)).await.is_err() {
                    recorder_open = false;
                }
                // Release any waiter even while the recorder is shutting down.
                latch.complete();
            }
            TransportDispatchJob::Response {
                forward,
                observation,
                predecessor,
            } => {
                predecessor.wait();
                if recorder_open
                    && sender
                        .send(RunEvent::Response(
                            forward,
                            observation,
                            CorrelationStatus::Correlated,
                        ))
                        .await
                        .is_err()
                {
                    recorder_open = false;
                }
            }
            #[cfg(test)]
            TransportDispatchJob::Placeholder { sequence: _, latch } => {
                latch.complete();
            }
        }
    }
}

/// Synchronous, bounded bridge from the proxy sinks to the run recorder.
///
/// Transport metadata uses one bounded FIFO and one fixed worker. Both halves of a forward share
/// that FIFO, so accepted transport status always reaches the recorder before its response.
#[derive(Debug)]
pub(super) struct RecorderSink {
    pub(super) durable: Arc<DurableEventIngress<RunEvent>>,
    pub(super) ordering: Arc<TransportOrdering>,
    pub(super) counters: Arc<CorrelationCounters>,
    pub(super) context_counters: Arc<ContextCounters>,
    /// Bounds analyzed outcomes retained between the recorder channel and the context worker.
    pub(super) analysis_slots: Arc<AnalysisOutputSlots>,
    pub(super) analysis_enabled: bool,
    pub(super) active_counters: Arc<ActiveCompressionCounters>,
}

impl RecorderSink {
    /// Admits transport metadata without allocating a per-forward task.
    fn offer_transport(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        if matches!(metadata.route, InboundRoute::ResponsesCompact) {
            // Compaction has its own terminal transport observation; creating a generic
            // inference here would double-count the provider request.
            return Ok(());
        }
        let forward = metadata.forward;
        if self.ordering.try_enqueue_transport(metadata) {
            return Ok(());
        }
        let reason = CorrelationDegradation::InFlightLimit;
        self.counters.degraded(reason);
        // Preserve a content-free degradation marker for routes without a semantic response.
        let _ = self
            .durable
            .try_send(RunEvent::TransportAdmissionFailed(forward, reason));
        Err(MetadataSinkError::rejected())
    }

    /// Queues an accepted response behind transport, or immediately records it as degraded.
    fn offer_ordered(
        &self,
        forward: ForwardId,
        observation: ResponseObservation,
    ) -> Result<(), MetadataSinkError> {
        match self
            .ordering
            .try_enqueue_response(forward, Box::new(observation))
        {
            Ok(()) => Ok(()),
            Err(failure) => {
                let reason = CorrelationDegradation::InFlightLimit;
                if failure.count {
                    self.counters.degraded(reason);
                }
                let _ = self
                    .durable
                    .try_send(RunEvent::TransportAdmissionFailed(forward, reason));
                self.durable
                    .try_send(RunEvent::Response(
                        forward,
                        failure.observation,
                        CorrelationStatus::Degraded(reason),
                    ))
                    .map_err(|_error| MetadataSinkError::rejected())
            }
        }
    }

    /// Provider observations are produced by detached observer tasks. An analyzed outcome may
    /// wait for a bounded context-ingestion slot here, but this method is never called on the
    /// forwarding task, so the wait cannot affect forwarding.
    fn offer_durable(&self, event: RunEvent) -> Result<(), MetadataSinkError> {
        self.durable.try_send(event)
    }

    fn offer_request(&self, event: RunEvent) -> Result<(), MetadataSinkError> {
        self.durable.send_request(event)
    }
}

impl MetadataSink for RecorderSink {
    fn try_record(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        self.offer_transport(metadata)
    }
}

impl ProviderObservationSink for RecorderSink {
    fn try_record_request_context(
        &self,
        observation: RequestContextObservation,
    ) -> Result<(), ObservationSinkError> {
        let forward = observation.forward;
        let fallback_reason = observation.context.as_ref().map_or(
            ContextAnalysisDropReason::CorrelationDegraded,
            |outcome| match outcome {
                ContextAnalysisOutcome::Analyzed(_) => {
                    ContextAnalysisDropReason::CorrelationDegraded
                }
                ContextAnalysisOutcome::Dropped(reason) => *reason,
                _ => ContextAnalysisDropReason::Unsupported,
            },
        );
        match self.offer_request(RunEvent::RequestContext(Box::new(observation))) {
            Ok(()) => Ok(()),
            Err(_error) => {
                if self.analysis_enabled {
                    self.context_counters.ensure_seen(forward);
                    self.context_counters.dropped(forward, fallback_reason);
                }
                Err(ObservationSinkError::rejected())
            }
        }
    }

    fn try_record_context_analysis(
        &self,
        observation: ContextAnalysisObservation,
    ) -> Result<(), ObservationSinkError> {
        let forward = observation.forward;
        if self.analysis_enabled {
            self.context_counters.ensure_seen(forward);
        }
        let permit = match &observation.outcome {
            ContextAnalysisOutcome::Analyzed(_) => Some(Arc::clone(&self.analysis_slots).acquire()),
            _ => None,
        };
        match self.offer_durable(RunEvent::ContextAnalysis(
            observation.forward,
            observation.outcome,
            observation.shadow_body,
            permit,
        )) {
            Ok(()) => Ok(()),
            Err(_error) => {
                if self.analysis_enabled {
                    self.context_counters
                        .dropped(forward, ContextAnalysisDropReason::ObserverBackpressure);
                }
                Err(ObservationSinkError::rejected())
            }
        }
    }

    fn try_record_active_compression(
        &self,
        observation: tracepress_proxy::ActiveCompressionObservation,
    ) -> Result<(), ObservationSinkError> {
        self.active_counters.record(&observation);
        Ok(())
    }

    fn try_record_response(
        &self,
        forward: ForwardId,
        observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        self.offer_ordered(forward, observation)
            .map_err(|_error| ObservationSinkError::rejected())
    }

    fn try_record_transport_failure(
        &self,
        forward: ForwardId,
        failure: TransportFailure,
    ) -> Result<(), ObservationSinkError> {
        self.ordering.release_transport(forward);
        self.offer_durable(RunEvent::TransportFailure(forward, failure))
            .map_err(|_error| ObservationSinkError::rejected())
    }

    fn try_record_compaction(
        &self,
        observation: CompactionObservation,
    ) -> Result<(), ObservationSinkError> {
        self.offer_durable(RunEvent::Compaction(Box::new(observation)))
            .map_err(|_error| {
                self.counters.compaction_dropped();
                ObservationSinkError::rejected()
            })
    }
}
