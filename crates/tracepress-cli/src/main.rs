#![allow(
    clippy::multiple_crate_versions,
    clippy::match_wildcard_for_single_variants,
    clippy::unnecessary_wraps,
    clippy::use_debug,
    clippy::format_collect,
    clippy::indexing_slicing,
    clippy::map_unwrap_or,
    clippy::print_stdout,
    clippy::unused_async,
    reason = "CLI boundary formats user-facing output and validates bounded fixed-size state"
)]
//! Tracepress command-line boundary.
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::serve;
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracepress_context::{
    ContextAnalysisLimits, ContextAnalysisResult, ContextAnalysisStatus, ContextBlockKind,
    ContextBlockSummary, ContextDeltaRequest, ContextOrigin, ContextRole, TokenEstimateAggregate,
    TokenReconciliation, compute_context_delta,
};
use tracepress_core::{
    AttemptId, ContextSnapshotId, HttpStatusCode, MaxIpcFrameBytes, MaxRequestBodyBytes,
    MaxResponseBodyBytes, OperationId, RequestId, ResourceLimits, ResourceLimitsConfig, SessionId,
    UuidV7Generator,
};
use tracepress_daemon::{
    ContextAnalysisFinalize, ContextAnalysisMetrics, ContextCorrelationStatusWire, ControlRequest,
    ControlResponse, CorrelationDegradation, CorrelationStatus,
    ProviderObservation as ObservationRecord, ProviderObservationOutcome,
};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcLimits, IpcRequest, ResponseOutcome, UnixEndpoint,
};
use tracepress_provider::{
    ProviderEndpoint, ProviderResponseState, RequestObservation, ResponseObservation,
};
use tracepress_proxy::{
    BackgroundTaskSpawner, ContextAnalysisDropReason, ContextAnalysisMode,
    ContextAnalysisObservation, ContextAnalysisOutcome, ForwardId, ForwardMetadata, InboundRoute,
    MetadataSink, MetadataSinkError, ObservationSinkError, ProviderObservationSink, ProxyConfig,
    RequestContextObservation, TransparentProxy, TransportFailure,
};
use tracepress_storage::{
    ContextInspection, ContextInspectionBlock, ContextInspectionNamedEstimate,
};

const FRAME_BYTES: u64 = 65_536;

/// Bounded queue of transport and semantic records awaiting durable recording.
const RECORDER_QUEUE_ITEMS: usize = 128;

/// Explicit cap for heavy context-ingestion jobs. One item may contain thousands of blocks, so
/// this queue is intentionally much smaller than the IPC item-count queue.
const CONTEXT_INGESTION_QUEUE_HARD_CAP: usize = 4;

fn context_ingestion_queue_capacity(configured: usize) -> usize {
    configured.clamp(1, CONTEXT_INGESTION_QUEUE_HARD_CAP)
}

/// Bound on forwards whose transport and semantic halves are still being correlated.
const IN_FLIGHT_FORWARDS: usize = 64;

/// Bound on retired identities kept one by one above the retirement watermark.
///
/// The watermark absorbs the contiguous prefix of retired identities for free, so this bound is
/// only reached when that many identities the proxy never reported an event for sit below the
/// newest retirement. Beyond it the oldest retirement is forgotten, which is the one least
/// likely to still have a half in flight.
const RETIRED_IDENTITIES: usize = 256;

/// Maximum total wait, after the agent and proxy have stopped, for provider observers, the
/// recorder, and context ingestion to finish.
const RECORDER_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Correlation accounting of one run, published while the recorder is still working.
///
/// The recorder owns bounded state, so evidence it cannot join is lost by design. Every counter
/// here names one such loss, and they live behind a shared handle so the run can report them
/// even when the bounded drain window expired before the recorder finished.
#[derive(Debug, Default)]
struct CorrelationCounters {
    /// Every degradation of this run, whatever its reason.
    degraded_total: AtomicU64,
    /// Correlation state evicted before settling because the in-flight bound was reached.
    degraded_inflight_limit: AtomicU64,
    /// Halves refused because the bounded retirement memory still accounts for their identity.
    degraded_retired_limit: AtomicU64,
    /// Terminal semantic evidence that reached the recorder without its request half.
    missing_total: AtomicU64,
}

impl CorrelationCounters {
    /// Counts one degradation under its reason and in the run total.
    fn degraded(&self, reason: CorrelationDegradation) {
        let counter = match reason {
            CorrelationDegradation::InFlightLimit => Some(&self.degraded_inflight_limit),
            CorrelationDegradation::RetiredLimit => Some(&self.degraded_retired_limit),
            CorrelationDegradation::MissingRequestHalf => Some(&self.missing_total),
            // A reason this build keeps no counter for still counts in the run total.
            _ => None,
        };
        if let Some(counter) = counter {
            let _counted = counter.fetch_add(1, Ordering::Relaxed);
        }
        let _total = self.degraded_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Reports every counter, including the zeroes.
    ///
    /// A run reports its correlation accounting unconditionally: a line printed only when
    /// something degraded cannot distinguish a clean run from a report that never arrived.
    fn report(&self) -> String {
        format!(
            "correlation_degraded_total={} correlation_degraded_inflight_limit={} correlation_degraded_retired_limit={} correlation_missing_total={}",
            self.degraded_total.load(Ordering::Relaxed),
            self.degraded_inflight_limit.load(Ordering::Relaxed),
            self.degraded_retired_limit.load(Ordering::Relaxed),
            self.missing_total.load(Ordering::Relaxed),
        )
    }
}
/// One auxiliary record of a single forward, tagged with its correlation identity.
#[derive(Debug)]
enum RunEvent {
    /// Allowlisted transport facts, observed once per forward.
    Transport(ForwardMetadata),
    RequestContext(Box<RequestContextObservation>),
    /// The detached Phase 3 outcome for a previously parsed request.
    ContextAnalysis(ForwardId, ContextAnalysisOutcome),
    /// The interpreted response of one forward.
    Response(ForwardId, Box<ResponseObservation>),
    /// The classified transport failure of a forward that obtained no upstream response.
    TransportFailure(ForwardId, TransportFailure),
}

/// Context data handed from the provider recorder to the independent context worker.
///
/// Provider persistence has already returned all three durable identities before this value is
/// admitted. The context worker therefore never needs to re-open, or infer, provider state.
#[derive(Debug)]
struct ContextIngestionJob {
    provider_request_id: RequestId,
    attempt_id: AttemptId,
    inference_operation_id: OperationId,
    analysis: ContextAnalysisResult,
    provider_input_tokens: Option<u64>,
    provider_usage_comparable: bool,
    correlation: CorrelationStatus,
}

/// Compact provider receipt retained while the detached context outcome is still in flight.
#[derive(Debug)]
struct ContextReceipt {
    provider_request_id: RequestId,
    attempt_id: AttemptId,
    inference_operation_id: OperationId,
    provider_input_tokens: Option<u64>,
    provider_usage_comparable: bool,
    correlation: CorrelationStatus,
}

struct ContextAppendInput<'analysis> {
    snapshot_id: ContextSnapshotId,
    analysis: &'analysis ContextAnalysisResult,
    status: ContextAnalysisStatus,
}

/// Accounting for context analyses rejected before context persistence.
#[derive(Debug, Default)]
struct ContextCounters {
    observer_backpressure: AtomicU64,
    resource_limit: AtomicU64,
    malformed: AtomicU64,
    correlation_degraded: AtomicU64,
    unsupported: AtomicU64,
    cancelled: AtomicU64,
}

impl ContextCounters {
    const fn counter(&self, reason: ContextAnalysisDropReason) -> &AtomicU64 {
        match reason {
            ContextAnalysisDropReason::ObserverBackpressure => &self.observer_backpressure,
            ContextAnalysisDropReason::ResourceLimit => &self.resource_limit,
            ContextAnalysisDropReason::Malformed => &self.malformed,
            ContextAnalysisDropReason::CorrelationDegraded => &self.correlation_degraded,
            ContextAnalysisDropReason::Cancelled => &self.cancelled,
            _ => &self.unsupported,
        }
    }

    fn dropped(&self, reason: ContextAnalysisDropReason) {
        let _counted = self.counter(reason).fetch_add(1, Ordering::Relaxed);
    }

    fn count(&self, reason: ContextAnalysisDropReason) -> u64 {
        self.counter(reason).load(Ordering::Relaxed)
    }

    fn report(&self) -> String {
        format!(
            "context_observer_backpressure_total={}\ncontext_resource_limit_total={}\ncontext_malformed_total={}\ncontext_correlation_degraded_total={}\ncontext_unsupported_total={}\ncontext_cancelled_total={}",
            self.observer_backpressure.load(Ordering::Relaxed),
            self.resource_limit.load(Ordering::Relaxed),
            self.malformed.load(Ordering::Relaxed),
            self.correlation_degraded.load(Ordering::Relaxed),
            self.unsupported.load(Ordering::Relaxed),
            self.cancelled.load(Ordering::Relaxed),
        )
    }
}

/// Coordinates the detached transport handoff with its terminal provider observation.
///
/// Each metadata event is submitted before the matching response observer can run. A response
/// observer that races the metadata worker waits on this latch, preserving the queue's event order
/// without making the forwarding task wait for recorder capacity.
#[derive(Debug, Default)]
struct TransportOrdering {
    pending: Mutex<BTreeMap<ForwardId, Arc<TransportLatch>>>,
}

#[derive(Debug, Default)]
struct TransportLatch {
    complete: Mutex<bool>,
    wake: Condvar,
}

impl TransportOrdering {
    fn register(&self, forward: ForwardId) -> Arc<TransportLatch> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(
            pending
                .entry(forward)
                .or_insert_with(|| Arc::new(TransportLatch::default())),
        )
    }

    fn take(&self, forward: ForwardId) -> Option<Arc<TransportLatch>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&forward)
    }

    fn remove(&self, forward: ForwardId, latch: &Arc<TransportLatch>) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let remove = pending
            .get(&forward)
            .is_some_and(|current| Arc::ptr_eq(current, latch));
        if remove {
            let _ = pending.remove(&forward);
        }
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

/// Synchronous, non-blocking bridge from the proxy sinks to the run recorder.
///
/// Both halves of a forward share one queue, so the transport status of a forward always
/// reaches the recorder before that forward's terminal semantic record.
#[derive(Debug)]
struct RecorderSink {
    sender: tokio::sync::mpsc::Sender<RunEvent>,
    background: BackgroundTaskSpawner,
    ordering: Arc<TransportOrdering>,
}

impl RecorderSink {
    fn offer(&self, event: RunEvent) -> Result<(), MetadataSinkError> {
        self.sender
            .try_send(event)
            .map_err(|_error| MetadataSinkError::rejected())
    }

    /// Enqueues one transport event from a tracked blocking task so the forwarding task never
    /// waits for recorder capacity.
    fn offer_transport(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        let forward = metadata.forward;
        let latch = self.ordering.register(forward);
        let task_latch = Arc::clone(&latch);
        let ordering = Arc::clone(&self.ordering);
        let sender = self.sender.clone();
        if !self.background.spawn_blocking(move || {
            let _ = sender.blocking_send(RunEvent::Transport(metadata));
            task_latch.complete();
            ordering.remove(forward, &task_latch);
        }) {
            latch.complete();
            self.ordering.remove(forward, &latch);
            return Err(MetadataSinkError::rejected());
        }
        Ok(())
    }

    /// Preserves transport-before-response ordering without making the forwarding or observer
    /// runtime task wait. The one-shot wait and durable send are both covered by the shutdown
    /// tracker.
    fn offer_ordered(&self, forward: ForwardId, event: RunEvent) -> Result<(), MetadataSinkError> {
        let Some(latch) = self.ordering.take(forward) else {
            return self.offer_durable(event);
        };
        let task_latch = Arc::clone(&latch);
        let sender = self.sender.clone();
        if !self.background.spawn_blocking(move || {
            task_latch.wait();
            let _ = sender.blocking_send(event);
        }) {
            latch.complete();
            self.ordering.remove(forward, &latch);
            return Err(MetadataSinkError::rejected());
        }
        Ok(())
    }

    /// Provider observations are produced by detached observer tasks, so a full recorder queue
    /// may stall those tasks without stalling the forwarding path. Waiting here preserves every
    /// Phase 2 provider receipt instead of silently dropping an observation during a burst.
    fn offer_durable(&self, event: RunEvent) -> Result<(), MetadataSinkError> {
        self.sender
            .blocking_send(event)
            .map_err(|_error| MetadataSinkError::rejected())
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
        self.offer_durable(RunEvent::RequestContext(Box::new(observation)))
    }
    fn try_record_context_analysis(
        &self,
        observation: ContextAnalysisObservation,
    ) -> Result<(), ObservationSinkError> {
        self.offer(RunEvent::ContextAnalysis(
            observation.forward,
            observation.outcome,
        ))
    }

    fn try_record_response(
        &self,
        forward: ForwardId,
        observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        self.offer_ordered(forward, RunEvent::Response(forward, Box::new(observation)))
            .map_err(|_error| ObservationSinkError::rejected())
    }

    fn try_record_transport_failure(
        &self,
        forward: ForwardId,
        failure: TransportFailure,
    ) -> Result<(), ObservationSinkError> {
        self.offer_durable(RunEvent::TransportFailure(forward, failure))
            .map_err(|_error| ObservationSinkError::rejected())
    }
}

/// One semantic request observation awaiting its terminal response evidence.
#[derive(Debug)]
struct PendingObservation {
    request: RequestObservation,
    started_at: String,
}

/// Correlation state of one forward whose evidence is not yet settled.
#[derive(Debug, Default)]
struct ForwardState {
    request: Option<PendingObservation>,
    response: Option<ResponseObservation>,
    context: Option<ContextAnalysisOutcome>,
    /// A Phase 2 request event arrived before its detached Phase 3 outcome.
    context_pending: bool,
    status_code: Option<u16>,
    transport_failure: Option<TransportFailure>,
    transport: bool,
    settled: bool,
}

impl ForwardState {
    /// Takes the evidence observed for this forward so far.
    ///
    /// A response without its request half carries no logical request, so it is dropped in
    /// favour of whatever transport evidence the forward has, and the forward reports that its
    /// correlation is missing rather than presenting transport evidence as a whole exchange.
    fn evidence(&mut self, forward: ForwardId) -> ForwardEvidence {
        let status_code = self.status_code;
        let response = self.response.take();
        let context = self.context.take();
        let transport_failure = self.transport_failure.take();
        let orphaned_semantic =
            self.request.is_none() && (response.is_some() || transport_failure.is_some());
        ForwardEvidence {
            forward,
            semantic: self.request.take().map(|pending| SemanticRecord {
                pending,
                response,
                context,
                status_code,
                transport_failure,
            }),
            orphaned_semantic,
            transport: self.transport,
        }
    }
}
/// Everything one settled forward contributes to durable storage.
#[derive(Debug)]
struct ForwardEvidence {
    forward: ForwardId,
    semantic: Option<SemanticRecord>,
    orphaned_semantic: bool,
    transport: bool,
}

/// Semantic evidence of one forward, with the transport evidence its forwarding half observed.
#[derive(Debug)]
struct SemanticRecord {
    pending: PendingObservation,
    response: Option<ResponseObservation>,
    context: Option<ContextAnalysisOutcome>,
    status_code: Option<u16>,
    transport_failure: Option<TransportFailure>,
}
struct RecordObservationInput {
    forward: ForwardId,
    semantic: SemanticRecord,
    correlation: CorrelationStatus,
}

/// Joins the transport and semantic records of every forward by correlation identity.
///
/// The proxy tags both halves of one forward with a [`ForwardId`], so each forward yields
/// exactly one inference operation: an observed Responses forward is owned by its semantic
/// record, which carries the upstream status the transport half observed, while every other
/// route keeps its transport-only record. Correlation state is bounded; when the bound is
/// reached the oldest identity is settled with the evidence it already has, and that identity
/// is retired so a half that arrives afterwards cannot open a second operation for a forward
/// already recorded.
#[derive(Debug)]
struct RunRecorder {
    config: Config,
    session_id: SessionId,
    parent_operation_id: OperationId,
    context_sender: tokio::sync::mpsc::Sender<ContextIngestionJob>,
    context_counters: Arc<ContextCounters>,
    forwards: BTreeMap<ForwardId, ForwardState>,
    /// Terminal response halves retained briefly when a request parser finishes after eviction.
    retired_orphans: BTreeMap<ForwardId, ForwardState>,
    /// Bounded receipts waiting for a detached context outcome.
    pending_context: BTreeMap<ForwardId, ContextReceipt>,
    /// Identities dropped from correlation state that the watermark does not cover yet.
    ///
    /// Bounded by [`RETIRED_IDENTITIES`]; its contiguous prefix is folded into the watermark.
    retired: BTreeSet<ForwardId>,
    /// Watermark at or below which every identity has already been retired, once one exists.
    ///
    /// Identities are monotonic per proxy, so a contiguous run of retired identities collapses
    /// into one watermark: refusing everything at or below it refuses only forwards this
    /// recorder has already accounted for.
    retired_through: Option<ForwardId>,
    /// Correlation losses observed so far, shared with the run that reports them.
    counters: Arc<CorrelationCounters>,
}

impl RunRecorder {
    async fn run(
        mut self,
        mut events: tokio::sync::mpsc::Receiver<RunEvent>,
    ) -> Result<(), String> {
        while let Some(event) = events.recv().await {
            match event {
                RunEvent::Transport(metadata) => self.transport(metadata).await?,
                RunEvent::RequestContext(observation) => {
                    self.request_context(*observation).await;
                }
                RunEvent::ContextAnalysis(forward, outcome) => {
                    self.context_analysis(forward, outcome).await;
                }
                RunEvent::Response(forward, observation) => {
                    self.response(forward, *observation).await;
                }
                RunEvent::TransportFailure(forward, failure) => {
                    self.transport_failure(forward, failure).await;
                }
            }
        }
        // A forward still in flight when the run ends is recorded with what it has.
        while let Some((forward, mut state)) = self.forwards.pop_first() {
            self.retire(forward);
            if state.settled {
                continue;
            }
            if state.request.is_none() {
                self.drop_unpaired_context(&mut state);
            }
            let evidence = state.evidence(forward);
            // The run ending is not a bound degradation: the forward keeps the evidence it has.
            self.record(evidence, CorrelationStatus::Correlated).await;
        }
        Ok(())
    }

    /// Records an unobserved route's forward, or joins the status onto a Responses forward.
    async fn transport(&mut self, metadata: ForwardMetadata) -> Result<(), String> {
        if !matches!(metadata.route, InboundRoute::Responses) {
            // Every route without semantic observation keeps its transport-only inference.
            return match self.record_forward().await? {
                ControlResponse::Ok { .. } => Ok(()),
                ControlResponse::Error { message } => Err(message),
                ControlResponse::Context { .. } => {
                    Err("daemon returned an unexpected context response".to_owned())
                }
            };
        }
        let evidence = {
            self.admit(metadata.forward).await;
            let Some(state) = self.forwards.get_mut(&metadata.forward) else {
                return Ok(());
            };
            if state.settled {
                return Ok(());
            }
            state.transport = true;
            state.status_code = metadata.status_code;
            if state.request.is_none() || state.response.is_none() {
                return Ok(());
            }
            // Metadata is handed off from a tracked blocking task. A response observer can
            // therefore reach the recorder first; settle only once both terminal halves and
            // transport status are present so successful attempts keep their status code.
            state.settled = true;
            state.evidence(metadata.forward)
        };
        self.record(evidence, CorrelationStatus::Correlated).await;
        Ok(())
    }

    /// Buffers the Phase 2 request half before dispatch and waits for its detached Phase 3 outcome
    /// before checking terminal evidence.
    async fn request_context(&mut self, observation: RequestContextObservation) {
        let forward = observation.forward;
        let Ok(started_at) = current_timestamp() else {
            return;
        };
        if let Some(mut state) = self.retired_orphans.remove(&forward) {
            if state.settled {
                return;
            }
            state.request = Some(PendingObservation {
                request: observation.observation,
                started_at,
            });
            state.context = observation.context;
            state.context_pending = state.context.is_none();
            state.settled = true;
            let evidence = state.evidence(forward);
            self.record(evidence, CorrelationStatus::Correlated).await;
            return;
        }
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        state.request = Some(PendingObservation {
            request: observation.observation,
            started_at,
        });
        state.context = observation.context;
        state.context_pending = state.context.is_none();
        if state.response.is_none() && state.transport_failure.is_none() {
            return;
        }
        state.settled = true;
        let evidence = state.evidence(forward);
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Attaches the detached Phase 3 outcome after the forwarding task has crossed dispatch.
    async fn context_analysis(&mut self, forward: ForwardId, outcome: ContextAnalysisOutcome) {
        if let Some(receipt) = self.pending_context.remove(&forward) {
            self.enqueue_context(&receipt, outcome);
            return;
        }
        let Some(state) = self.forwards.get_mut(&forward) else {
            self.context_counters
                .dropped(ContextAnalysisDropReason::CorrelationDegraded);
            return;
        };
        if state.settled {
            self.context_counters
                .dropped(ContextAnalysisDropReason::CorrelationDegraded);
            return;
        }
        state.context = Some(outcome);
        state.context_pending = false;
        if state.request.is_none() || state.response.is_none() && state.transport_failure.is_none()
        {
            return;
        }
        state.settled = true;
        let evidence = state.evidence(forward);
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Buffers the terminal semantic response half, settling the forward once paired.
    ///
    /// The forwarding half reports transport metadata from a detached task, so it may still be
    /// in flight here; successful responses wait for that status before they are persisted.
    async fn response(&mut self, forward: ForwardId, observation: ResponseObservation) {
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        state.response = Some(observation);
        if state.request.is_none() || !state.transport {
            return;
        }
        state.settled = true;
        let evidence = state.evidence(forward);
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Settles a forward that obtained no upstream response with its transport evidence.
    ///
    /// The forwarding half reports the failure from its own task, so the request half may still
    /// be in flight here; the forward then waits for it and its detached context outcome.
    async fn transport_failure(&mut self, forward: ForwardId, failure: TransportFailure) {
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        state.transport_failure = Some(failure);
        if state.request.is_none() {
            return;
        }
        state.settled = true;
        let evidence = state.evidence(forward);
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Reserves bounded correlation state for one forward identity.
    ///
    /// A settled identity is kept until eviction, and eviction retires it, so a half that
    /// arrives after its forward was recorded is refused instead of re-admitting a blank state
    /// that could produce a second inference operation for the same forward. An identity that
    /// never held correlation state is admitted whatever its ordinal: identities are allocated
    /// before a body is read, so a forward whose bounded parse is expensive reaches the recorder
    /// after cheaper, newer forwards have already evicted state, and refusing it would leave the
    /// forward with no record at all.
    async fn admit(&mut self, forward: ForwardId) {
        if self.forwards.contains_key(&forward) {
            return;
        }
        if self.is_retired(forward) {
            // This forward was already recorded with the evidence it had, so this half can no
            // longer be joined to it and re-admitting it would record the forward twice. The
            // evidence is lost either way; what must not be lost is that it was.
            self.degrade(CorrelationDegradation::RetiredLimit).await;
            return;
        }
        while self.forwards.len() >= IN_FLIGHT_FORWARDS {
            // Identities are monotonic, so the first key is deterministically the oldest.
            let Some((oldest, mut state)) = self.forwards.pop_first() else {
                break;
            };
            self.retire(oldest);
            if state.settled {
                continue;
            }
            if state.request.is_none() {
                self.drop_unpaired_context(&mut state);
            }
            if state.request.is_none()
                && (state.response.is_some() || state.transport_failure.is_some())
            {
                // Terminal semantic evidence without a request cannot become a provider row.
                // Persist its transport-only operation now, then retain a settled tombstone so
                // the late parser half is consumed rather than re-admitting this identity.
                let evidence = state.evidence(oldest);
                self.record(
                    evidence,
                    CorrelationStatus::Degraded(CorrelationDegradation::InFlightLimit),
                )
                .await;
                let _previous = self.retired_orphans.insert(
                    oldest,
                    ForwardState {
                        settled: true,
                        ..state
                    },
                );
                while self.retired_orphans.len() > RETIRED_IDENTITIES {
                    let _forgotten = self.retired_orphans.pop_first();
                }
                continue;
            }
            let evidence = state.evidence(oldest);
            // The bound, not the exchange, ended this forward's correlation: it is recorded
            // with the evidence it has and marked degraded, never as a whole exchange.
            self.record(
                evidence,
                CorrelationStatus::Degraded(CorrelationDegradation::InFlightLimit),
            )
            .await;
        }
        let _admitted = self.forwards.insert(forward, ForwardState::default());
    }

    /// Reports whether this identity already held correlation state and lost it.
    fn is_retired(&self, forward: ForwardId) -> bool {
        self.retired_through
            .is_some_and(|watermark| forward <= watermark)
            || self.retired.contains(&forward)
    }

    /// Retires one identity that correlation state no longer holds.
    fn retire(&mut self, forward: ForwardId) {
        if self.is_retired(forward) {
            return;
        }
        let _retired = self.retired.insert(forward);
        // Identities start at zero and are monotonic, so a contiguous run of retirements is
        // exactly what the watermark states and costs nothing to keep past the fold.
        while let Some(smallest) = self.retired.first().copied() {
            let next = self
                .retired_through
                .map_or(0, |watermark| watermark.get().saturating_add(1));
            if smallest.get() != next {
                break;
            }
            let _folded = self.retired.pop_first();
            self.retired_through = Some(smallest);
        }
        // An identity the proxy never reported an event for — an oversized request body is
        // rejected before any observation — blocks that fold for the rest of the run, so the
        // retirements stranded above it are bounded rather than kept forever.
        while self.retired.len() > RETIRED_IDENTITIES {
            let _forgotten = self.retired.pop_first();
        }
    }

    /// Records the single inference operation one settled forward is entitled to.
    ///
    /// Semantic evidence owns that operation whenever the record reaches the daemon, including
    async fn record(&mut self, evidence: ForwardEvidence, correlation: CorrelationStatus) {
        let ForwardEvidence {
            forward,
            semantic,
            orphaned_semantic,
            transport,
        } = evidence;
        if let CorrelationStatus::Degraded(reason) = correlation {
            self.counters.degraded(reason);
        }
        // The record carries its own correlation status, so the daemon commits the degradation
        // event in the very transaction that persists the evidence.
        let recorded = match semantic {
            Some(semantic) => {
                self.record_observation(RecordObservationInput {
                    forward,
                    semantic,
                    correlation,
                })
                .await
            }
            None => false,
        };
        if !recorded {
            // No record reached the daemon to carry the degradation, so it is reported alone.
            if let CorrelationStatus::Degraded(reason) = correlation {
                self.report_degradation(reason).await;
            }
            if transport {
                // Fallback transport evidence is auxiliary: its failure never fails the run.
                drop(self.record_forward().await);
            }
        }
        if orphaned_semantic {
            // Terminal evidence without a request half is a degradation of its own, whatever
            // else happened to this forward: no logical request can be recorded for it.
            self.degrade(CorrelationDegradation::MissingRequestHalf)
                .await;
        }
    }

    /// Counts one degradation no forward record can carry and makes it observable.
    async fn degrade(&self, reason: CorrelationDegradation) {
        self.counters.degraded(reason);
        self.report_degradation(reason).await;
    }

    /// Asks the daemon to commit the canonical event of one correlation degradation.
    ///
    /// The message carries the reason, the session, and the moment only: a degradation is
    /// missing evidence, so it may not carry the forward it degraded.
    async fn report_degradation(&self, reason: CorrelationDegradation) {
        let Ok(observed_at) = current_timestamp() else {
            return;
        };
        let _ = control(
            &self.config,
            ControlRequest::RecordCorrelationDegradation {
                session_id: self.session_id,
                reason,
                observed_at,
            },
        )
        .await;
    }

    async fn record_observation(&mut self, input: RecordObservationInput) -> bool {
        let RecordObservationInput {
            forward,
            semantic,
            correlation,
        } = input;
        let Ok(ended_at) = current_timestamp() else {
            return false;
        };
        let Some((observation, context, provider_input, provider_usage_comparable)) =
            observation_record(semantic, ended_at, correlation)
        else {
            return false;
        };
        let response = control(
            &self.config,
            ControlRequest::RecordProviderObservation {
                session_id: self.session_id,
                parent_operation_id: self.parent_operation_id,
                observation: Box::new(observation),
            },
        )
        .await;
        let Ok(ControlResponse::Ok {
            provider_request_id: Some(provider_request_id),
            attempt_id: Some(attempt_id),
            inference_operation_id: Some(inference_operation_id),
            ..
        }) = response
        else {
            return false;
        };
        let receipt = ContextReceipt {
            provider_request_id,
            attempt_id,
            inference_operation_id,
            provider_input_tokens: provider_input,
            provider_usage_comparable,
            correlation,
        };
        match context {
            Some(outcome) => self.enqueue_context(&receipt, outcome),
            None => {
                if self.pending_context.len() >= IN_FLIGHT_FORWARDS {
                    self.context_counters
                        .dropped(ContextAnalysisDropReason::ObserverBackpressure);
                } else {
                    let _previous = self.pending_context.insert(forward, receipt);
                }
            }
        }
        true
    }

    fn enqueue_context(&self, receipt: &ContextReceipt, outcome: ContextAnalysisOutcome) {
        match outcome {
            ContextAnalysisOutcome::Analyzed(analysis) => {
                let job = ContextIngestionJob {
                    provider_request_id: receipt.provider_request_id,
                    attempt_id: receipt.attempt_id,
                    inference_operation_id: receipt.inference_operation_id,
                    analysis,
                    provider_input_tokens: receipt.provider_input_tokens,
                    provider_usage_comparable: receipt.provider_usage_comparable,
                    correlation: receipt.correlation,
                };
                if self.context_sender.try_send(job).is_err() {
                    self.context_counters
                        .dropped(ContextAnalysisDropReason::ObserverBackpressure);
                }
            }
            ContextAnalysisOutcome::Dropped(reason) => self.context_counters.dropped(reason),
            _ => self
                .context_counters
                .dropped(ContextAnalysisDropReason::Unsupported),
        }
    }

    fn drop_unpaired_context(&self, state: &mut ForwardState) {
        let Some(outcome) = state.context.take() else {
            return;
        };
        let reason = match outcome {
            ContextAnalysisOutcome::Analyzed(_analysis) => {
                ContextAnalysisDropReason::CorrelationDegraded
            }
            ContextAnalysisOutcome::Dropped(reason) => reason,
            _ => ContextAnalysisDropReason::Unsupported,
        };
        self.context_counters.dropped(reason);
    }

    /// Records one transport-only inference operation beneath the agent root.
    async fn record_forward(&self) -> Result<ControlResponse, String> {
        control(
            &self.config,
            ControlRequest::RecordForward {
                session_id: self.session_id,
                parent_operation_id: self.parent_operation_id,
                observed_at: current_timestamp()?,
            },
        )
        .await
    }
}

/// The bounded predecessor retained by the context worker for the next context delta.
#[derive(Debug)]
struct PreviousContextSnapshot {
    id: ContextSnapshotId,
    blocks: Vec<ContextBlockSummary>,
}

/// Owns every potentially slow context IPC operation.
///
/// The provider recorder never waits on this worker: it only admits a compact job with the
/// provider receipt identities already returned by `RecordProviderObservation`.
#[derive(Debug)]
struct ContextIngestionWorker {
    config: Config,
    session_id: SessionId,
    context_analysis_limits: ContextAnalysisLimits,
    previous_context: Option<PreviousContextSnapshot>,
    counters: Arc<ContextCounters>,
}

impl ContextIngestionWorker {
    async fn run(
        mut self,
        mut jobs: tokio::sync::mpsc::Receiver<ContextIngestionJob>,
    ) -> Result<(), String> {
        while let Some(job) = jobs.recv().await {
            self.record_context(job).await;
        }
        self.flush_drops().await;
        Ok(())
    }

    async fn flush_drops(&self) {
        let reasons = [
            ContextAnalysisDropReason::ObserverBackpressure,
            ContextAnalysisDropReason::ResourceLimit,
            ContextAnalysisDropReason::Malformed,
            ContextAnalysisDropReason::CorrelationDegraded,
            ContextAnalysisDropReason::Unsupported,
            ContextAnalysisDropReason::Cancelled,
        ];
        let Ok(observed_at_us) = current_timestamp_us() else {
            return;
        };
        for reason in reasons {
            let dropped = self.counters.count(reason);
            if dropped == 0 {
                continue;
            }
            let _ = control(
                &self.config,
                ControlRequest::RecordContextAnalysisDropped {
                    session_id: self.session_id,
                    reason,
                    dropped,
                    observed_at_us,
                },
            )
            .await;
        }
    }

    async fn begin_context(
        &self,
        provider_request_id: RequestId,
        inference_operation_id: OperationId,
    ) -> Option<(ContextSnapshotId, u64)> {
        let started_at_us = current_timestamp_us().ok()?;
        let response = control(
            &self.config,
            ControlRequest::BeginContextAnalysis {
                session_id: self.session_id,
                provider_request_id,
                inference_operation_id,
                analysis_version: tracepress_context::CONTEXT_ANALYSIS_VERSION,
                started_at_us,
            },
        )
        .await
        .ok()?;
        match response {
            ControlResponse::Ok {
                context_snapshot_id: Some(snapshot_id),
                ..
            } => Some((snapshot_id, started_at_us)),
            _ => None,
        }
    }
    async fn append_context_blocks(
        &self,
        input: ContextAppendInput<'_>,
    ) -> (ContextAnalysisStatus, u64) {
        let ContextAppendInput {
            snapshot_id,
            analysis,
            mut status,
        } = input;
        let mut sequence = 0_u32;
        let max_batches =
            u32::try_from(self.context_analysis_limits.max_batches.get()).unwrap_or(u32::MAX);
        let body_limit = usize::try_from(BODY_BYTES).unwrap_or(0);
        let mut batch = Vec::new();
        let mut accepted_block_count = 0_u64;
        for block in analysis.blocks.iter().cloned() {
            if sequence >= max_batches {
                status = ContextAnalysisStatus::Partial;
                break;
            }
            let mut candidate = batch.clone();
            candidate.push(block.clone());
            let request = ControlRequest::AppendContextBlocks {
                snapshot_id,
                sequence,
                blocks: candidate,
            };
            let Ok(encoded) = serde_json::to_vec(&request) else {
                status = ContextAnalysisStatus::Partial;
                break;
            };
            if encoded.len() <= body_limit {
                batch.push(block);
                continue;
            }
            if batch.is_empty() {
                // A single block that cannot fit is dropped; it is never truncated into a
                // superficially complete snapshot.
                status = ContextAnalysisStatus::Partial;
                break;
            }
            let outgoing = std::mem::take(&mut batch);
            let outgoing_len = u64::try_from(outgoing.len()).unwrap_or(u64::MAX);
            if control(
                &self.config,
                ControlRequest::AppendContextBlocks {
                    snapshot_id,
                    sequence,
                    blocks: outgoing,
                },
            )
            .await
            .is_err()
            {
                status = ContextAnalysisStatus::Partial;
                break;
            }
            accepted_block_count = accepted_block_count.saturating_add(outgoing_len);
            sequence = sequence.saturating_add(1);
            if sequence >= max_batches {
                status = ContextAnalysisStatus::Partial;
                break;
            }
            let singleton = vec![block.clone()];
            let singleton_request = ControlRequest::AppendContextBlocks {
                snapshot_id,
                sequence,
                blocks: singleton,
            };
            let Ok(singleton_encoded) = serde_json::to_vec(&singleton_request) else {
                status = ContextAnalysisStatus::Partial;
                break;
            };
            if singleton_encoded.len() > body_limit {
                status = ContextAnalysisStatus::Partial;
                break;
            }
            batch.push(block);
        }
        if !batch.is_empty() {
            if sequence >= max_batches {
                status = ContextAnalysisStatus::Partial;
            } else {
                let outgoing_len = u64::try_from(batch.len()).unwrap_or(u64::MAX);
                if control(
                    &self.config,
                    ControlRequest::AppendContextBlocks {
                        snapshot_id,
                        sequence,
                        blocks: batch,
                    },
                )
                .await
                .is_err()
                {
                    status = ContextAnalysisStatus::Partial;
                } else {
                    accepted_block_count = accepted_block_count.saturating_add(outgoing_len);
                }
            }
        }
        (status, accepted_block_count)
    }

    async fn record_context(&mut self, job: ContextIngestionJob) {
        let ContextIngestionJob {
            provider_request_id,
            attempt_id,
            inference_operation_id,
            analysis,
            provider_input_tokens,
            provider_usage_comparable,
            correlation,
        } = job;
        let Some((snapshot_id, started_at_us)) = self
            .begin_context(provider_request_id, inference_operation_id)
            .await
        else {
            return;
        };

        let current_blocks = context_block_summaries(&analysis);
        let delta = self.previous_context.as_ref().map(|previous| {
            compute_context_delta(ContextDeltaRequest {
                previous_snapshot_id: previous.id,
                current_snapshot_id: snapshot_id,
                previous: &previous.blocks,
                current: &current_blocks,
                limits: self.context_analysis_limits,
            })
        });
        let (metrics, visible_estimated_tokens) = context_metrics(
            &analysis,
            delta
                .as_ref()
                .and_then(|value| value.common_prefix_estimated_tokens),
        );

        let mut status = analysis.status;
        let correlation_status = if matches!(correlation, CorrelationStatus::Correlated) {
            ContextCorrelationStatusWire::Correlated
        } else {
            status = ContextAnalysisStatus::CorrelationDegraded;
            ContextCorrelationStatusWire::Degraded
        };
        let (status, accepted_block_count) = self
            .append_context_blocks(ContextAppendInput {
                snapshot_id,
                analysis: &analysis,
                status,
            })
            .await;

        let reconciliation = TokenReconciliation::reconcile(
            snapshot_id,
            analysis.visibility,
            visible_estimated_tokens,
            provider_input_tokens,
            provider_usage_comparable
                && matches!(
                    status,
                    ContextAnalysisStatus::Complete
                        | ContextAnalysisStatus::Partial
                        | ContextAnalysisStatus::ResourceLimit
                ),
        );
        let Ok(finalize) = ContextAnalysisFinalize::builder(
            snapshot_id,
            status,
            current_timestamp_us().unwrap_or(started_at_us),
        )
        .request_content_hash(Some(analysis.request_content_hash))
        .explicit_block_count(Some(accepted_block_count))
        .analyzed_bytes(Some(analysis.analyzed_bytes))
        .skipped_bytes(Some(analysis.skipped_bytes))
        .visibility(analysis.visibility)
        .duplicate_key_detected(Some(analysis.duplicate_key_detected))
        .reference_resolved_locally(Some(analysis.visibility_facts.reference_resolved_locally))
        .correlation_status(correlation_status)
        .metrics(metrics)
        .delta(delta)
        .reconciliation(reconciliation)
        .attempt_id(Some(attempt_id))
        .build() else {
            return;
        };
        let finalized = control(
            &self.config,
            ControlRequest::FinalizeContextAnalysis {
                summary: Box::new(finalize),
            },
        )
        .await;
        if matches!(finalized, Ok(ControlResponse::Ok { .. })) {
            self.previous_context = Some(PreviousContextSnapshot {
                id: snapshot_id,
                blocks: current_blocks,
            });
        }
    }
}

fn observation_record(
    semantic: SemanticRecord,
    ended_at: String,
    correlation: CorrelationStatus,
) -> Option<(
    ObservationRecord,
    Option<ContextAnalysisOutcome>,
    Option<u64>,
    bool,
)> {
    let SemanticRecord {
        pending,
        response,
        context,
        status_code,
        transport_failure,
        ..
    } = semantic;
    // The parser always measures its bounded input; an unmeasured body is never invented.
    let request_bytes = pending.request.request_bytes?;
    let streaming = pending.request.stream;
    // An unobserved or unrepresentable status stays unknown rather than fabricated.
    let status_code = status_code.and_then(|value| HttpStatusCode::new(value).ok());
    // An errored upstream status is terminal evidence by itself: the exchange is over even when
    // nothing interpreted the body, which is what an upstream answering a media type the
    // observer does not read leaves behind. Closing the attempt here keeps the timestamp a real
    // local measurement instead of one the daemon would have to invent, and keeps the daemon
    // from persisting an errored attempt whose `ended_at` is still NULL.
    let errored_upstream = status_code.is_some_and(|code| !(200..300).contains(&code.get()));
    let terminal = response.is_some() || transport_failure.is_some() || errored_upstream;
    let provider_input_tokens = response
        .as_ref()
        .and_then(|value| value.normalized_usage.as_ref())
        .and_then(|usage| usage.input_total);
    let provider_usage_comparable = response.as_ref().is_some_and(|value| {
        value.usage_status == tracepress_provider::UsageStatus::Final
            && value
                .normalized_usage
                .as_ref()
                .is_some_and(|usage| !usage.anomalies.any())
    });
    let mut record = ObservationRecord::new(request_bytes, pending.request, pending.started_at)
        .with_correlation_status(correlation);
    if let Some(streaming) = streaming {
        record = record.with_streaming(streaming);
    }
    if let Some(status_code) = status_code {
        record = record.with_status_code(status_code);
    }
    if let Some(response) = response {
        let outcome = observation_outcome(response.response_state);
        record = record.with_response(response).with_outcome(outcome);
    }
    // The failure class is content-free evidence that this forward reached no response at all,
    // so it overrides any semantic outcome and closes the attempt.
    if let Some(failure) = transport_failure {
        record = record.with_transport_error(failure.label());
    }
    if terminal {
        record = record.with_ended_at(ended_at);
    }
    Some((
        record,
        context,
        provider_input_tokens,
        provider_usage_comparable,
    ))
}

fn context_block_summaries(analysis: &ContextAnalysisResult) -> Vec<ContextBlockSummary> {
    analysis
        .blocks
        .iter()
        .map(|block| ContextBlockSummary {
            exact_fingerprint: block.exact_fingerprint,
            semantic_fingerprint: block.semantic_fingerprint,
            estimated_tokens: block
                .token_estimate
                .as_ref()
                .map(|estimate| estimate.tokens),
        })
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "one bounded pass keeps all context metric precedence together"
)]
#[allow(
    clippy::cast_precision_loss,
    reason = "bounded context token totals are intentionally projected to metric ratios"
)]
fn context_metrics(
    analysis: &ContextAnalysisResult,
    stable_explicit_prefix_estimate: Option<u64>,
) -> (ContextAnalysisMetrics, Option<u64>) {
    let analysis_complete = matches!(analysis.status, ContextAnalysisStatus::Complete);
    let mut all = TokenEstimateAggregate::new();
    let mut by_kind: [TokenEstimateAggregate; 15] =
        std::array::from_fn(|_| TokenEstimateAggregate::new());
    let mut by_role: [TokenEstimateAggregate; 6] =
        std::array::from_fn(|_| TokenEstimateAggregate::new());
    let mut by_origin: [TokenEstimateAggregate; 8] =
        std::array::from_fn(|_| TokenEstimateAggregate::new());
    let mut confidence = None;
    let mut mixed_confidence = false;
    let mut opportunity_signals = Vec::new();
    let mut exact_groups: HashMap<_, (u64, TokenEstimateAggregate)> = HashMap::new();
    let mut schema_groups: HashMap<_, (u64, TokenEstimateAggregate)> = HashMap::new();
    let mut tool_count = 0_u64;
    let mut schema_bytes = 0_u64;
    let mut largest_tool_schema = 0_u64;
    let mut schema_aggregate = TokenEstimateAggregate::new();

    for block in &analysis.blocks {
        let estimate = block.token_estimate.as_ref();
        all.observe_estimate(estimate);
        by_kind[context_kind_index(block.kind)].observe_estimate(estimate);
        if let Some(role) = block.role {
            by_role[context_role_index(role)].observe_estimate(estimate);
        }
        by_origin[context_origin_index(block.origin)].observe_estimate(estimate);
        if let Some(estimate) = estimate {
            match confidence {
                None => confidence = Some(estimate.confidence),
                Some(previous) if previous != estimate.confidence => mixed_confidence = true,
                Some(_) => {}
            }
        }
        for signal in block.opportunity_signals.iter() {
            if !opportunity_signals.contains(&signal) {
                opportunity_signals.push(signal);
            }
        }
        let exact_entry = exact_groups
            .entry(block.exact_fingerprint)
            .or_insert_with(|| (0, TokenEstimateAggregate::new()));
        exact_entry.0 = exact_entry.0.saturating_add(1);
        exact_entry.1.observe_estimate(estimate);
        if matches!(block.kind, ContextBlockKind::ToolDefinition) {
            tool_count = tool_count.saturating_add(1);
            schema_bytes = schema_bytes.saturating_add(block.raw_bytes);
            largest_tool_schema = largest_tool_schema.max(block.raw_bytes);
            schema_aggregate.observe_estimate(estimate);
            let schema_entry = schema_groups
                .entry(block.exact_fingerprint)
                .or_insert_with(|| (0, TokenEstimateAggregate::new()));
            schema_entry.0 = schema_entry.0.saturating_add(1);
            schema_entry.1.observe_estimate(estimate);
        }
    }

    let aggregate_total = |aggregate: &TokenEstimateAggregate| {
        if aggregate.observed_blocks() == 0 && !analysis_complete {
            None
        } else {
            aggregate.complete_total()
        }
    };
    let visible_estimated_tokens = aggregate_total(&all);
    let kind_totals = std::array::from_fn(|index| aggregate_total(&by_kind[index]));
    let role_totals = std::array::from_fn(|index| aggregate_total(&by_role[index]));
    let origin_totals = std::array::from_fn(|index| aggregate_total(&by_origin[index]));

    let mut unique_total = Some(0_u64);
    let mut repeated_total = Some(0_u64);
    let mut saw_unique = false;
    let mut saw_repeated = false;
    for (count, aggregate) in exact_groups.values() {
        let total = aggregate.complete_total();
        if *count == 1 {
            saw_unique = true;
            let Some(value) = total else {
                unique_total = None;
                continue;
            };
            if let Some(sum) = unique_total.as_mut() {
                *sum = sum.saturating_add(value);
            }
        } else {
            saw_repeated = true;
            let Some(value) = total else {
                repeated_total = None;
                continue;
            };
            if let Some(sum) = repeated_total.as_mut() {
                *sum = sum.saturating_add(value);
            }
        }
    }
    let share = |part: Option<u64>| {
        visible_estimated_tokens.and_then(|total| {
            if total == 0 {
                None
            } else {
                part.map(|value| value as f64 / total as f64)
            }
        })
    };

    let mut repeated_schema_tokens = Some(0_u64);
    let mut saw_repeated_schema = false;
    for (count, aggregate) in schema_groups.values() {
        if *count <= 1 {
            continue;
        }
        saw_repeated_schema = true;
        let Some(value) = aggregate.complete_total() else {
            repeated_schema_tokens = None;
            continue;
        };
        if let Some(sum) = repeated_schema_tokens.as_mut() {
            *sum = sum.saturating_add(value);
        }
    }
    let estimated_schema_tokens = if tool_count == 0 {
        None
    } else {
        aggregate_total(&schema_aggregate)
    };
    let tool_count = if tool_count == 0 && !analysis_complete {
        None
    } else {
        Some(tool_count)
    };
    let schema_bytes = if schema_bytes == 0 && !analysis_complete {
        None
    } else {
        Some(schema_bytes)
    };
    let largest_tool_schema = if largest_tool_schema == 0 && !analysis_complete {
        None
    } else {
        Some(largest_tool_schema)
    };
    let repeated_schema_tokens = if saw_repeated_schema {
        repeated_schema_tokens
    } else if analysis_complete {
        Some(0)
    } else {
        None
    };
    let opportunity_signals = if opportunity_signals.is_empty() && !analysis_complete {
        None
    } else {
        Some(opportunity_signals)
    };

    let mut metrics = ContextAnalysisMetrics::new();
    metrics.explicit_bytes = Some(analysis.analyzed_bytes);
    metrics.estimated_tokens_by_kind = kind_totals;
    metrics.estimated_tokens_by_role = role_totals;
    metrics.estimated_tokens_by_origin = origin_totals;
    metrics.estimated_tool_definition_share = share(aggregate_total(
        &by_kind[context_kind_index(ContextBlockKind::ToolDefinition)],
    ));
    metrics.estimated_tool_result_share = share(aggregate_total(
        &by_kind[context_kind_index(ContextBlockKind::ToolResult)],
    ));
    metrics.estimated_human_text_share = share(aggregate_total(
        &by_origin[context_origin_index(ContextOrigin::HumanAuthored)],
    ));
    metrics.estimated_assistant_history_share = share(aggregate_total(
        &by_kind[context_kind_index(ContextBlockKind::AssistantHistory)],
    ));
    metrics.estimated_unique_content_share = if saw_unique {
        share(unique_total)
    } else if analysis_complete {
        Some(0.0)
    } else {
        None
    };
    metrics.estimated_repeated_content_share = if saw_repeated {
        share(repeated_total)
    } else if analysis_complete {
        Some(0.0)
    } else {
        None
    };
    metrics.tool_count = tool_count;
    metrics.schema_bytes = schema_bytes;
    metrics.estimated_schema_tokens = estimated_schema_tokens;
    metrics.largest_tool_schema = largest_tool_schema;
    metrics.repeated_schema_tokens = repeated_schema_tokens;
    metrics.stable_explicit_prefix_estimate = stable_explicit_prefix_estimate;
    metrics.estimator = all.estimator().map(|value| value.as_str().to_owned());
    metrics.estimator_version = all.estimator_version();
    metrics.estimate_confidence = if mixed_confidence { None } else { confidence };
    metrics.opportunity_signals = opportunity_signals;
    (metrics, visible_estimated_tokens)
}

const fn context_kind_index(kind: ContextBlockKind) -> usize {
    match kind {
        ContextBlockKind::Instructions => 0,
        ContextBlockKind::Message => 1,
        ContextBlockKind::Text => 2,
        ContextBlockKind::ImageReference => 3,
        ContextBlockKind::FileReference => 4,
        ContextBlockKind::ToolDefinition => 5,
        ContextBlockKind::ToolCall => 6,
        ContextBlockKind::ToolResult => 7,
        ContextBlockKind::ItemReference => 8,
        ContextBlockKind::PromptReference => 9,
        ContextBlockKind::ProviderStateReference => 10,
        ContextBlockKind::AssistantHistory => 11,
        ContextBlockKind::OpaqueReasoning => 12,
        ContextBlockKind::Opaque => 13,
        _ => 14,
    }
}

const fn context_role_index(role: ContextRole) -> usize {
    match role {
        ContextRole::System => 0,
        ContextRole::Developer => 1,
        ContextRole::User => 2,
        ContextRole::Assistant => 3,
        ContextRole::Tool => 4,
        _ => 5,
    }
}

const fn context_origin_index(origin: ContextOrigin) -> usize {
    match origin {
        ContextOrigin::HumanAuthored => 0,
        ContextOrigin::AgentGenerated => 1,
        ContextOrigin::ToolGenerated => 2,
        ContextOrigin::ToolSchema => 3,
        ContextOrigin::ProviderManaged => 4,
        ContextOrigin::ExternalReference => 5,
        ContextOrigin::TracepressGenerated => 6,
        _ => 7,
    }
}
const fn observation_outcome(state: ProviderResponseState) -> ProviderObservationOutcome {
    match state {
        ProviderResponseState::Completed => ProviderObservationOutcome::Completed,
        ProviderResponseState::Incomplete => ProviderObservationOutcome::Incomplete,
        ProviderResponseState::Failed => ProviderObservationOutcome::Failed,
        ProviderResponseState::Cancelled => ProviderObservationOutcome::Cancelled,
        ProviderResponseState::Disconnected => ProviderObservationOutcome::Disconnected,
        _ => ProviderObservationOutcome::InProgress,
    }
}

/// Session identities a recorder attributes one run's evidence to.
struct RunRecording {
    config: Config,
    session_id: SessionId,
    parent_operation_id: OperationId,
    context_queue_items: usize,
    context_analysis_limits: ContextAnalysisLimits,
    previous_context: Option<PreviousContextSnapshot>,
}
/// The recorder of one run: its proxy, its workers, and the counters it publishes.
struct SpawnedRecorder {
    proxy: TransparentProxy,
    recorder_task: tokio::task::JoinHandle<Result<(), String>>,
    context_task: tokio::task::JoinHandle<Result<(), String>>,
    /// Correlation accounting the run reports, readable whether or not the workers finished.
    counters: Arc<CorrelationCounters>,
    /// Context analyses rejected by the bounded context queue.
    context_counters: Arc<ContextCounters>,
}

/// Installs both auxiliary sinks and starts the provider and context workers.
fn spawn_recorder(recording: RunRecording, proxy: TransparentProxy) -> SpawnedRecorder {
    let RunRecording {
        config,
        session_id,
        parent_operation_id,
        context_queue_items,
        context_analysis_limits,
        previous_context,
    } = recording;
    let (sender, receiver) = tokio::sync::mpsc::channel(RECORDER_QUEUE_ITEMS);
    let context_queue_items = context_ingestion_queue_capacity(context_queue_items);
    let (context_sender, context_receiver) = tokio::sync::mpsc::channel(context_queue_items);
    let counters = Arc::new(CorrelationCounters::default());
    let context_counters = Arc::new(ContextCounters::default());
    let context_task = tokio::spawn(
        ContextIngestionWorker {
            config: config.clone(),
            session_id,
            context_analysis_limits,
            previous_context,
            counters: Arc::clone(&context_counters),
        }
        .run(context_receiver),
    );
    let recorder_task = tokio::spawn(
        RunRecorder {
            config,
            session_id,
            parent_operation_id,
            context_sender,
            context_counters: Arc::clone(&context_counters),
            forwards: BTreeMap::new(),
            retired_orphans: BTreeMap::new(),
            pending_context: BTreeMap::new(),
            retired: BTreeSet::new(),
            retired_through: None,
            counters: Arc::clone(&counters),
        }
        .run(receiver),
    );
    let background = proxy.background_task_spawner();
    let ordering = Arc::new(TransportOrdering::default());
    let metadata_sink = Arc::new(RecorderSink {
        sender: sender.clone(),
        background: background.clone(),
        ordering: Arc::clone(&ordering),
    });
    let observation_sink = Arc::new(RecorderSink {
        sender,
        background,
        ordering,
    });
    let proxy = proxy
        .with_metadata_sink(metadata_sink)
        .with_observation_sink(observation_sink);
    SpawnedRecorder {
        proxy,
        recorder_task,
        context_task,
        counters,
        context_counters,
    }
}

const BODY_BYTES: u64 = 32_768;

#[derive(Debug, Parser)]
#[command(
    name = "tracepress",
    version,
    about = "Bounded Tracepress local runtime"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    Init,
    Doctor,
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Run {
        agent: String,
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    Context {
        request_id: RequestId,
    },
    Proxy,
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    Start,
    Stop,
    Status,
}

#[derive(Clone, Debug)]
struct Config {
    root: PathBuf,
    database: PathBuf,
    socket: PathBuf,
    credential: PathBuf,
    ready: PathBuf,
}

impl Config {
    fn load() -> Result<Self, String> {
        let root = std::env::var_os("TRACEPRESS_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".tracepress"));
        Ok(Self {
            database: root.join("tracepress.sqlite3"),
            socket: root.join("tracepress.sock"),
            credential: root.join("control.cred"),
            ready: root.join("daemon.ready"),
            root,
        })
    }
    fn ensure_root(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("cannot create {}: {e}", self.root.display()))?;
        #[cfg(unix)]
        std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot secure {}: {e}", self.root.display()))?;
        Ok(())
    }
}

fn limits() -> Result<IpcLimits, String> {
    Ok(IpcLimits::new(
        MaxIpcFrameBytes::new(FRAME_BYTES).map_err(|e| e.to_string())?,
        MaxRequestBodyBytes::new(BODY_BYTES).map_err(|e| e.to_string())?,
        MaxResponseBodyBytes::new(BODY_BYTES).map_err(|e| e.to_string())?,
    ))
}
fn credential(config: &Config) -> Result<Credential, String> {
    let text = std::fs::read_to_string(&config.credential)
        .map_err(|e| format!("cannot read credential: {e}"))?;
    let text = text.trim();
    if text.len() != 64 {
        return Err("credential must contain exactly 32 bytes".to_owned());
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| "credential is not UTF-8")?,
            16,
        )
        .map_err(|_| "credential is not hexadecimal")?;
    }
    Ok(Credential::new(bytes))
}

async fn control(config: &Config, request: ControlRequest) -> Result<ControlResponse, String> {
    let client = IpcClient::authenticated(
        Endpoint::Unix(UnixEndpoint::new(config.socket.clone()).map_err(|e| e.to_string())?),
        credential(config)?,
        limits()?,
    );
    let cancellation = CancellationToken::new();
    let mut connection = client
        .connect(&cancellation)
        .await
        .map_err(|e| format!("daemon unavailable: {e}"))?;
    let ids = UuidV7Generator::new();
    let body = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let request = IpcRequest::new(
        RequestId::generate(&ids),
        body,
        MaxRequestBodyBytes::new(BODY_BYTES).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    connection
        .send_request(&request, &cancellation)
        .await
        .map_err(|e| e.to_string())?;
    let response = connection
        .receive_response(&cancellation)
        .await
        .map_err(|e| e.to_string())?;
    match response.outcome() {
        ResponseOutcome::Complete { body } => {
            serde_json::from_slice(body).map_err(|e| format!("invalid daemon response: {e}"))
        }
        ResponseOutcome::Incomplete | ResponseOutcome::Cancelled => {
            Err("daemon returned an incomplete response".to_owned())
        }
    }
}
async fn daemon_running(config: &Config) -> Result<bool, String> {
    if !config.socket.exists() {
        return Ok(false);
    }
    match tokio::net::UnixStream::connect(&config.socket).await {
        Ok(probe) => drop(probe),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(format!("cannot probe daemon socket: {error}")),
    }
    match control(config, ControlRequest::Status).await? {
        ControlResponse::Ok { .. } => Ok(true),
        ControlResponse::Error { message } => Err(message),
        ControlResponse::Context { .. } => {
            Err("daemon returned an unexpected context response".to_owned())
        }
    }
}
async fn init(config: &Config) -> Result<(), String> {
    config.ensure_root()?;
    if !config.credential.exists() {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut options = std::fs::OpenOptions::new();
        let _options = options.write(true).create_new(true);
        #[cfg(unix)]
        let _mode = options.mode(0o600);
        std::io::Write::write_all(
            &mut options
                .open(&config.credential)
                .map_err(|e| e.to_string())?,
            hex.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    }
    println!("initialized {}", config.root.display());
    Ok(())
}

async fn daemon_start(config: &Config) -> Result<(), String> {
    init(config).await?;
    if daemon_running(config).await? {
        return Err("daemon is already running".to_owned());
    }
    let daemon = std::env::var_os("TRACEPRESSD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tracepressd"));
    let mut child = Command::new(daemon)
        .env("TRACEPRESS_DATABASE", &config.database)
        .env("TRACEPRESS_CONTROL_SOCKET", &config.socket)
        .env("TRACEPRESS_CONTROL_CREDENTIAL", &config.credential)
        .env("TRACEPRESS_DAEMON_READY", &config.ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start tracepressd: {e}"))?;
    for _ in 0..50 {
        if config.ready.exists() {
            match daemon_running(config).await {
                Ok(true) => {
                    println!("daemon running");
                    return Ok(());
                }
                Ok(false) => {}
                Err(error) => {
                    let _killed = child.kill().await;
                    return Err(format!("daemon readiness check failed: {error}"));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = child.kill().await;
    Err("daemon did not become ready within 5 seconds".to_owned())
}

async fn daemon_stop(config: &Config) -> Result<(), String> {
    let response = control(config, ControlRequest::Shutdown).await?;
    match response {
        ControlResponse::Ok { .. } => {}
        ControlResponse::Error { message } => return Err(message),
        ControlResponse::Context { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    }
    for _ in 0..50 {
        if !config.socket.exists() {
            let _removed = std::fs::remove_file(&config.ready);
            println!("daemon stopped");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("daemon did not stop within 5 seconds".to_owned())
}

fn proxy_resource_limits(
    request_bytes: u64,
    response_bytes: u64,
) -> Result<ResourceLimits, String> {
    ResourceLimits::try_from(ResourceLimitsConfig {
        max_raw_bytes: Some(i128::from(request_bytes)),
        max_request_body_bytes: Some(i128::from(request_bytes)),
        max_response_body_bytes: Some(i128::from(response_bytes)),
        max_decompressed_bytes: Some(i128::from(response_bytes)),
        max_ipc_frame_bytes: Some(i128::from(FRAME_BYTES)),
        max_ipc_queue_items: Some(i128::from(RECORDER_QUEUE_ITEMS as u64)),
        max_json_nesting: Some(64),
        max_json_items: Some(100_000),
        max_line_bytes: Some(i128::from(request_bytes)),
        max_processing_time_ms: Some(250),
        max_cpu_work_units: Some(1_000_000),
    })
    .map_err(|error| error.to_string())
}

fn context_analysis_mode() -> Result<ContextAnalysisMode, String> {
    let value = std::env::var("TRACEPRESS_CONTEXT_ANALYSIS")
        .unwrap_or_else(|_| "shadow".to_owned())
        .to_ascii_lowercase();
    match value.as_str() {
        "off" => Ok(ContextAnalysisMode::Off),
        "shadow" => Ok(ContextAnalysisMode::Shadow),
        _ => Err(format!(
            "TRACEPRESS_CONTEXT_ANALYSIS must be `off` or `shadow`, got `{value}`"
        )),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the existing run path keeps lifecycle cleanup and reporting ordered"
)]
async fn run_agent(config: &Config, agent: String, args: Vec<String>) -> Result<(), String> {
    if !daemon_running(config).await? {
        return Err("daemon is not running; run `tracepress daemon start` first".to_owned());
    }
    let upstream = std::env::var("TRACEPRESS_UPSTREAM")
        .map_err(|_| "TRACEPRESS_UPSTREAM is required by `tracepress run`")?;
    let endpoint = ProviderEndpoint::new(&upstream).map_err(|error| error.to_string())?;
    let resource_limits = proxy_resource_limits(8 * 1024 * 1024, 32 * 1024 * 1024)?;
    let analysis_mode = context_analysis_mode()?;
    let context_queue_items = usize::try_from(resource_limits.max_ipc_queue_items.get())
        .map_err(|error| format!("context queue capacity does not fit usize: {error}"))?;
    let context_analysis_limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)
        .map_err(|error| error.to_string())?;
    let proxy = TransparentProxy::new(
        ProxyConfig::new(endpoint, resource_limits, analysis_mode)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let proxy_address = listener.local_addr().map_err(|error| error.to_string())?;
    let started_at = current_timestamp()?;
    let response = control(config, ControlRequest::StartSession { started_at }).await?;
    let (session, parent_operation_id) = match response {
        ControlResponse::Ok {
            session: Some(session),
            operation_id: Some(operation_id),
            ..
        } => (session, operation_id),
        ControlResponse::Error { message } => return Err(message),
        ControlResponse::Ok { .. } => {
            return Err("daemon did not return a session and root operation".to_owned());
        }
        ControlResponse::Context { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    };
    let SpawnedRecorder {
        proxy,
        mut recorder_task,
        mut context_task,
        counters,
        context_counters,
    } = spawn_recorder(
        RunRecording {
            config: config.clone(),
            session_id: session.session_id,
            parent_operation_id,
            context_queue_items,
            context_analysis_limits,
            previous_context: None,
        },
        proxy,
    );
    let background_proxy = proxy.clone();
    let proxy_task = tokio::spawn(async move { serve(listener, proxy.router()).await });
    let base_url = format!("http://{proxy_address}/v1");
    let status_result = Command::new(&agent)
        .args(args)
        .env("TRACEPRESS_SESSION_ID", session.session_id.to_string())
        .env("OPENAI_BASE_URL", &base_url)
        .env(
            "TRACEPRESS_PROXY_URL",
            format!("http://{proxy_address}/v1/chat/completions"),
        )
        .env(
            "TRACEPRESS_RESPONSES_URL",
            format!("http://{proxy_address}/v1/responses"),
        )
        .status()
        .await;
    proxy_task.abort();
    let _proxy_result = proxy_task.await;
    let recorded = match tokio::time::timeout(RECORDER_DRAIN_TIMEOUT, async {
        background_proxy.wait_for_background_tasks().await;
        // The recorder's channel must be closed before it is drained; otherwise the recorder
        // cannot observe end-of-run and wait for more events forever.
        drop(background_proxy);
        drain_recorders(&mut recorder_task, &mut context_task).await
    })
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            recorder_task.abort();
            context_task.abort();
            Err(format!(
                "provider observer/recorder drain exceeded {}s",
                RECORDER_DRAIN_TIMEOUT.as_secs()
            ))
        }
    };
    // Correlation degradation is reported for every run, before anything else can fail: a run
    // whose bounded state lost evidence must never look like a run that lost none.
    println!("{}", counters.report());
    println!("{}", context_counters.report());
    let ended_at = current_timestamp()?;
    let finalization = control(
        config,
        ControlRequest::FinishSession {
            session_id: session.session_id,
            ended_at,
        },
    )
    .await;
    let status = status_result.map_err(|error| format!("cannot launch agent {agent}: {error}"))?;
    recorded.map_err(|error| format!("forward recording failed: {error}"))?;
    match finalization? {
        ControlResponse::Ok { .. } => {}
        ControlResponse::Error { message } => {
            return Err(format!(
                "agent finished but session finalization failed: {message}"
            ));
        }
        ControlResponse::Context { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    }
    match status.code() {
        Some(code) if code != 0 => Err(format!("agent exited with status {code}")),
        Some(_) => Ok(()),
        None => Err("agent terminated by signal".to_owned()),
    }
}

async fn drain_recorders(
    recorder_task: &mut tokio::task::JoinHandle<Result<(), String>>,
    context_task: &mut tokio::task::JoinHandle<Result<(), String>>,
) -> Result<(), String> {
    let recorder_result = (&mut *recorder_task)
        .await
        .map_err(|error| format!("recording worker failed: {error}"))
        .and_then(|result| result);
    // Always wait for the context worker after the recorder has stopped, even when the
    // provider worker reported an IPC error.
    let context_result = (&mut *context_task)
        .await
        .map_err(|error| format!("context worker failed: {error}"))
        .and_then(|result| result);
    recorder_result.and(context_result)
}

fn current_timestamp() -> Result<String, String> {
    Ok(format!(
        "unix-ms:{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis()
    ))
}
async fn proxy() -> Result<(), String> {
    let upstream = std::env::var("TRACEPRESS_UPSTREAM")
        .map_err(|_| "TRACEPRESS_UPSTREAM must be set to /v1/chat/completions")?;
    let endpoint = ProviderEndpoint::new(&upstream).map_err(|error| error.to_string())?;
    let resource_limits = proxy_resource_limits(8 * 1024 * 1024, 32 * 1024 * 1024)?;
    let analysis_mode = context_analysis_mode()?;
    let proxy = TransparentProxy::new(
        ProxyConfig::new(endpoint, resource_limits, analysis_mode)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listen =
        std::env::var("TRACEPRESS_PROXY_LISTEN").unwrap_or_else(|_| "127.0.0.1:0".to_owned());
    let listener = TcpListener::bind(&listen)
        .await
        .map_err(|error| error.to_string())?;
    println!(
        "proxy listening on {}",
        listener.local_addr().map_err(|error| error.to_string())?
    );
    serve(listener, proxy.router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| error.to_string())
}

async fn doctor(config: &Config) -> Result<(), String> {
    config.ensure_root()?;
    let _credential = credential(config)?;
    println!("state: {}", config.root.display());
    println!(
        "daemon: {}",
        if daemon_running(config).await? {
            "running"
        } else {
            "stopped"
        }
    );
    Ok(())
}

async fn context(config: &Config, request_id: RequestId) -> Result<(), String> {
    match control(config, ControlRequest::Context { request_id }).await? {
        ControlResponse::Context { inspection } => print_context_inspection(&inspection),
        ControlResponse::Error { message } => return Err(message),
        ControlResponse::Ok { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "the command intentionally renders the stable inspection sections together"
)]
fn print_context_inspection(inspection: &ContextInspection) {
    println!("Context visibility");
    println!("  request id: {}", inspection.request_id);
    println!("  snapshot id: {}", inspection.snapshot_id);
    println!(
        "  explicit request complete: {}",
        bool_text(inspection.visibility.explicit_request_complete)
    );
    println!(
        "  logical context status: {}",
        optional_text(inspection.visibility.logical_context_status.as_ref())
    );
    println!(
        "  previous response state: {}",
        bool_text(inspection.visibility.uses_previous_response)
    );
    println!(
        "  conversation state: {}",
        bool_text(inspection.visibility.uses_conversation_state)
    );
    println!(
        "  item references: {}",
        bool_text(inspection.visibility.uses_item_references)
    );
    println!(
        "  prompt reference: {}",
        bool_text(inspection.visibility.uses_prompt_reference)
    );
    println!(
        "  external files: {}",
        bool_text(inspection.visibility.uses_external_files)
    );
    println!(
        "  external images: {}",
        bool_text(inspection.visibility.uses_external_images)
    );
    println!(
        "  opaque items: {}",
        bool_text(inspection.visibility.contains_opaque_items)
    );
    println!(
        "  duplicate keys: {}",
        bool_text(inspection.visibility.duplicate_key_detected)
    );
    println!(
        "  references resolved locally: {}",
        bool_text(inspection.visibility.reference_resolved_locally)
    );

    println!("\nProvider input observed");
    println!(
        "  provider: {}",
        optional_text(inspection.provider.as_ref())
    );
    println!(
        "  protocol: {}",
        optional_text(inspection.protocol.as_ref())
    );
    println!("  model: {}", optional_text(inspection.model.as_ref()));
    println!(
        "  accepted request bytes: {}",
        optional_number(inspection.request_bytes)
    );
    println!(
        "  input tokens observed: {}",
        optional_number(inspection.provider_input_tokens)
    );

    println!("\nVisible estimate + estimator + reconciliation/residual");
    println!(
        "  visible tokens (estimated): {}",
        estimated_number(inspection.visible_estimated_tokens)
    );
    println!(
        "  estimator: {}",
        optional_text(inspection.estimator.as_ref())
    );
    println!(
        "  estimator version: {}",
        optional_number(inspection.estimator_version.map(u64::from))
    );
    println!(
        "  estimate confidence: {}",
        optional_text(inspection.estimate_confidence.as_ref())
    );
    println!(
        "  reconciliation status: {}",
        optional_text(inspection.reconciliation_status.as_ref())
    );
    println!(
        "  residual (provider minus visible estimate, signed): {}",
        signed_number(inspection.residual_tokens)
    );

    println!("\nComposition (estimated)");
    print_named_estimates("  by kind", &inspection.composition.by_kind);
    print_named_estimates("  by role", &inspection.composition.by_role);
    print_named_estimates("  by origin", &inspection.composition.by_origin);
    print_share(
        "  tool definition share (estimated)",
        inspection.composition.estimated_tool_definition_share,
    );
    print_share(
        "  tool result share (estimated)",
        inspection.composition.estimated_tool_result_share,
    );
    print_share(
        "  human text share (estimated)",
        inspection.composition.estimated_human_text_share,
    );
    print_share(
        "  assistant history share (estimated)",
        inspection.composition.estimated_assistant_history_share,
    );
    print_share(
        "  unique content share (estimated)",
        inspection.composition.estimated_unique_content_share,
    );
    print_share(
        "  repeated content share (estimated)",
        inspection.composition.estimated_repeated_content_share,
    );
    println!(
        "  tool count: {}",
        optional_number(inspection.composition.tool_count)
    );
    println!(
        "  schema bytes: {}",
        optional_number(inspection.composition.schema_bytes)
    );
    println!(
        "  schema tokens (estimated): {}",
        estimated_number(inspection.composition.estimated_schema_tokens)
    );
    println!(
        "  largest tool schema bytes: {}",
        optional_number(inspection.composition.largest_tool_schema)
    );
    println!(
        "  repeated schema tokens (estimated): {}",
        estimated_number(inspection.composition.repeated_schema_tokens)
    );
    println!(
        "  opportunity signals: {}",
        if inspection.composition.opportunity_signals.is_empty() {
            "none".to_owned()
        } else {
            inspection.composition.opportunity_signals.join(", ")
        }
    );

    println!("\nRepetition");
    println!(
        "  repeated blocks: {}",
        optional_number(inspection.repetition.repeated_blocks)
    );
    println!(
        "  new blocks: {}",
        optional_number(inspection.repetition.new_blocks)
    );
    println!(
        "  changed blocks: {}",
        optional_number(inspection.repetition.changed_blocks)
    );
    println!(
        "  removed blocks: {}",
        optional_number(inspection.repetition.removed_blocks)
    );
    println!(
        "  repeated tokens (estimated): {}",
        estimated_number(inspection.repetition.repeated_estimated_tokens)
    );
    println!(
        "  new tokens (estimated): {}",
        estimated_number(inspection.repetition.new_estimated_tokens)
    );

    println!("\nStable explicit prefix");
    println!(
        "  common-prefix blocks: {}",
        optional_number(inspection.repetition.common_prefix_blocks)
    );
    println!(
        "  common-prefix tokens (estimated): {}",
        estimated_number(inspection.repetition.common_prefix_estimated_tokens)
    );
    println!(
        "  stable explicit-prefix estimate (estimated): {}",
        estimated_number(inspection.stable_explicit_prefix_estimate)
    );

    println!("\nLargest blocks");
    if inspection.largest_blocks.is_empty() {
        println!("  unavailable");
    } else {
        for block in &inspection.largest_blocks {
            print_block(block);
        }
    }

    println!("\nAnalysis");
    println!("  status: {}", analysis_status_text(inspection));
    println!("  version: {}", inspection.analysis_version);
    println!(
        "  explicit blocks: {}",
        optional_number(inspection.coverage.explicit_block_count)
    );
    println!(
        "  analyzed bytes: {}",
        optional_number(inspection.coverage.analyzed_bytes)
    );
    println!(
        "  skipped bytes: {}",
        optional_number(inspection.coverage.skipped_bytes)
    );
    println!(
        "  explicit bytes: {}",
        optional_number(inspection.coverage.explicit_bytes)
    );

    println!("\nCorrelation");
    println!(
        "  context correlation: {}",
        optional_text(inspection.correlation_status.as_ref())
    );
    println!(
        "  attempt id: {}",
        inspection
            .attempt_id
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
    );
}

fn analysis_status_text(inspection: &ContextInspection) -> String {
    if inspection.analysis_status == "complete"
        && inspection
            .visibility
            .logical_context_status
            .as_deref()
            .is_some_and(|status| status.ends_with("_partial"))
    {
        "finalized (logical context partial)".to_owned()
    } else {
        inspection.analysis_status.clone()
    }
}

fn print_named_estimates(label: &str, values: &[ContextInspectionNamedEstimate]) {
    println!("{label}:");
    for value in values {
        println!(
            "    {}: {} estimated tokens",
            value.name,
            estimated_number(value.estimated_tokens)
        );
    }
}

fn print_share(label: &str, value: Option<f64>) {
    println!(
        "{label}: {}",
        value.map_or_else(|| "unknown".to_owned(), |value| format!("{value:.3}"))
    );
}

fn print_block(block: &ContextInspectionBlock) {
    println!(
        "  ordinal {}: kind={} role={} origin={} raw bytes={}",
        block.ordinal, block.kind, block.role, block.origin, block.raw_bytes
    );
    println!(
        "    estimated tokens: {}",
        estimated_number(block.estimated_tokens)
    );
    println!(
        "    detection: {}",
        optional_text(block.detected_kind.as_ref())
    );
    println!(
        "    detector confidence: {}",
        block
            .detector_confidence
            .map_or_else(|| "unknown".to_owned(), |value| format!("{value:.3}"))
    );
    println!(
        "    detector version: {}",
        optional_number(block.detector_version.map(u64::from))
    );
    println!(
        "    repetition score: {}",
        block
            .repetition_score
            .map_or_else(|| "unknown".to_owned(), |value| format!("{value:.3}"))
    );
    println!(
        "    opportunity signals: {}",
        if block.opportunity_signals.is_empty() {
            "none".to_owned()
        } else {
            block.opportunity_signals.join(", ")
        }
    );
    println!(
        "    candidate tokens (estimated): {}",
        estimated_number(block.candidate_estimated_tokens)
    );
}

fn optional_text(value: Option<&String>) -> String {
    value.cloned().unwrap_or_else(|| "unknown".to_owned())
}

fn optional_number(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

fn estimated_number(value: Option<u64>) -> String {
    optional_number(value)
}

fn signed_number(value: Option<i64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| format!("{value:+}"))
}

const fn bool_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

fn current_timestamp_us() -> Result<u64, String> {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_micros(),
    )
    .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let cli = Cli::parse();
    let config = Config::load()?;
    match cli.command {
        CommandKind::Init => init(&config).await,
        CommandKind::Doctor => doctor(&config).await,
        CommandKind::Daemon {
            command: DaemonCommand::Start,
        } => daemon_start(&config).await,
        CommandKind::Context { request_id } => context(&config, request_id).await,
        CommandKind::Daemon {
            command: DaemonCommand::Stop,
        } => daemon_stop(&config).await,
        CommandKind::Daemon {
            command: DaemonCommand::Status,
        } => {
            println!(
                "{}",
                if daemon_running(&config).await? {
                    "running"
                } else {
                    "stopped"
                }
            );
            Ok(())
        }
        CommandKind::Run { agent, args } => run_agent(&config, agent, args).await,
        CommandKind::Proxy => proxy().await,
    }
}
#[cfg(test)]
mod tests {
    use super::{CONTEXT_INGESTION_QUEUE_HARD_CAP, context_ingestion_queue_capacity};

    #[test]
    fn context_ingestion_queue_is_hard_capped_without_zero_capacity() {
        assert_eq!(context_ingestion_queue_capacity(0), 1);
        assert_eq!(context_ingestion_queue_capacity(1), 1);
        assert_eq!(
            context_ingestion_queue_capacity(CONTEXT_INGESTION_QUEUE_HARD_CAP),
            CONTEXT_INGESTION_QUEUE_HARD_CAP
        );
        assert_eq!(
            context_ingestion_queue_capacity(CONTEXT_INGESTION_QUEUE_HARD_CAP + 1),
            CONTEXT_INGESTION_QUEUE_HARD_CAP
        );
        assert_eq!(context_ingestion_queue_capacity(128), 4);
    }
}
