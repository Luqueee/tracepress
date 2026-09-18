//! Bounded recording pipeline and correlation lifecycle for one agent run.

use super::*;

pub(crate) mod compression;
pub(crate) mod context;
pub(crate) mod transport;

use compression::*;
use context::*;
use transport::*;

pub(super) const FRAME_BYTES: u64 = 65_536;

/// Bounded queue of transport and semantic records awaiting durable recording.
pub(super) const RECORDER_QUEUE_ITEMS: usize = 128;

/// Explicit cap for heavy context-ingestion jobs. One item may contain thousands of blocks, so
/// this queue is intentionally much smaller than the IPC item-count queue.
pub(super) const CONTEXT_INGESTION_QUEUE_HARD_CAP: usize = 4;

pub(super) fn context_ingestion_queue_capacity(configured: usize) -> usize {
    configured.clamp(1, CONTEXT_INGESTION_QUEUE_HARD_CAP)
}

/// Bound on forwards whose transport and semantic halves are still being correlated.
const IN_FLIGHT_FORWARDS: usize = 64;
/// Transport admission and dispatch share the existing 64-forward correlation bound.
pub(super) const TRANSPORT_DISPATCH_QUEUE_ITEMS: usize = IN_FLIGHT_FORWARDS;

/// Bound on retired identities kept one by one above the retirement watermark.
///
/// The watermark absorbs the contiguous prefix of retired identities for free, so this bound is
/// only reached when that many identities the proxy never reported an event for sit below the
/// newest retirement. Beyond it the oldest retirement is forgotten, which is the one least
/// likely to still have a half in flight.
const RETIRED_IDENTITIES: usize = 256;
/// Bounded tombstones prevent a late duplicate event from re-opening a terminal analysis.
pub(super) const TERMINAL_ANALYSIS_IDENTITIES: usize = 256;

/// Maximum total wait, after the agent and proxy have stopped, for provider observers, the
/// recorder, and context ingestion to finish.
pub(super) const RECORDER_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Correlation accounting of one run, published while the recorder is still working.
///
/// The recorder owns bounded state, so evidence it cannot join is lost by design. Every counter
/// here names one such loss, and they live behind a shared handle so the run can report them
/// even when the bounded drain window expired before the recorder finished.
#[derive(Debug, Default)]
pub(super) struct CorrelationCounters {
    /// Every degradation of this run, whatever its reason.
    degraded_total: AtomicU64,
    /// Correlation state evicted before settling because the in-flight bound was reached.
    degraded_inflight_limit: AtomicU64,
    /// Halves refused because the bounded retirement memory still accounts for their identity.
    degraded_retired_limit: AtomicU64,
    /// Terminal semantic evidence that reached the recorder without its request half.
    missing_total: AtomicU64,
    /// Compact transport evidence rejected after the recorder queue closed.
    compaction_dropped_total: AtomicU64,
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
    pub(super) fn report(&self) -> String {
        format!(
            "correlation_degraded_total={} correlation_degraded_inflight_limit={} correlation_degraded_retired_limit={} correlation_missing_total={}\ncompaction_observation_dropped_total={}",
            self.degraded_total.load(Ordering::Relaxed),
            self.degraded_inflight_limit.load(Ordering::Relaxed),
            self.degraded_retired_limit.load(Ordering::Relaxed),
            self.missing_total.load(Ordering::Relaxed),
            self.compaction_dropped_total.load(Ordering::Relaxed),
        )
    }

    fn compaction_dropped(&self) {
        let _counted = self
            .compaction_dropped_total
            .fetch_add(1, Ordering::Relaxed);
    }
}
/// One auxiliary record of a single forward, tagged with its correlation identity.
#[derive(Debug)]
enum RunEvent {
    /// Allowlisted transport facts, observed once per forward.
    Transport(ForwardMetadata),
    RequestContext(Box<RequestContextObservation>),
    /// The detached Phase 3 outcome for a previously parsed request.
    ContextAnalysis(
        ForwardId,
        ContextAnalysisOutcome,
        Option<ShadowAnalysisBody>,
        Option<AnalysisOutputPermit>,
    ),
    /// The interpreted response of one forward, with its correlation status.
    Response(ForwardId, Box<ResponseObservation>, CorrelationStatus),
    /// Transport admission failed before a metadata record could be queued.
    TransportAdmissionFailed(ForwardId, CorrelationDegradation),
    /// The classified transport failure of a forward that obtained no upstream response.
    TransportFailure(ForwardId, TransportFailure),
    /// Transport-only evidence for one provider-managed context compaction.
    Compaction(Box<CompactionObservation>),
}

/// Bounded ingress for durable semantic events.
///
/// Provider request/context events are produced by detached observers. A full recorder channel
/// must defer those compact events instead of rejecting them merely because the recorder is
/// temporarily busy with `SQLite` work. One tracked pump preserves the bound and avoids creating a
/// blocking task per event.
pub(super) struct DurableEventIngress<T> {
    sender: tokio::sync::mpsc::Sender<T>,
    pub(super) state: Mutex<DurableEventIngressState<T>>,
    pub(super) space: Condvar,
    background: BackgroundTaskSpawner,
    on_drop: Arc<dyn Fn(T) + Send + Sync>,
}

#[derive(Debug)]
pub(super) struct DurableEventIngressState<T> {
    pub(super) events: VecDeque<T>,
    closed: bool,
    pump_running: bool,
}

impl<T> Default for DurableEventIngressState<T> {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            closed: false,
            pump_running: false,
        }
    }
}

impl<T> std::fmt::Debug for DurableEventIngress<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableEventIngress")
            .field("sender", &self.sender)
            .finish_non_exhaustive()
    }
}

impl<T> DurableEventIngress<T>
where
    T: Send + 'static,
{
    pub(super) fn new(
        sender: tokio::sync::mpsc::Sender<T>,
        background: BackgroundTaskSpawner,
        on_drop: impl Fn(T) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            sender,
            state: Mutex::new(DurableEventIngressState::default()),
            space: Condvar::new(),
            background,
            on_drop: Arc::new(on_drop),
        })
    }

    pub(super) fn try_send(self: &Arc<Self>, event: T) -> Result<(), MetadataSinkError> {
        let start_pump = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed || state.events.len() >= RECORDER_QUEUE_ITEMS {
                return Err(MetadataSinkError::rejected());
            }
            state.events.push_back(event);
            if state.pump_running {
                false
            } else {
                state.pump_running = true;
                true
            }
        };
        if start_pump {
            let ingress = Arc::clone(self);
            let _spawned = self.background.spawn(async move {
                ingress.run().await;
            });
        }
        Ok(())
    }

    /// Sends a request observation through the bounded handoff, waiting for queue space only on
    /// the detached parser worker. Forwarding never calls this method, so provider admission can
    /// preserve its request identity without making the agent wait on the recorder.
    pub(super) fn send_request(self: &Arc<Self>, event: T) -> Result<(), MetadataSinkError> {
        let start_pump = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while !state.closed && state.events.len() >= RECORDER_QUEUE_ITEMS {
                state = self
                    .space
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if state.closed {
                return Err(MetadataSinkError::rejected());
            }
            state.events.push_back(event);
            if state.pump_running {
                false
            } else {
                state.pump_running = true;
                true
            }
        };
        if start_pump {
            let ingress = Arc::clone(self);
            let _spawned = self.background.spawn(async move {
                ingress.run().await;
            });
        }
        Ok(())
    }

    async fn run(self: Arc<Self>) {
        loop {
            let event = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(event) = state.events.pop_front() else {
                    state.pump_running = false;
                    self.space.notify_all();
                    return;
                };
                self.space.notify_one();
                event
            };
            if let Err(error) = self.sender.send(event).await {
                let mut dropped = vec![error.0];
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.closed = true;
                dropped.extend(state.events.drain(..));
                state.pump_running = false;
                self.space.notify_all();
                drop(state);
                for event in dropped {
                    (self.on_drop)(event);
                }
                return;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn queued_len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .events
            .len()
    }
}

const fn dropped_context_reason(
    outcome: Option<&ContextAnalysisOutcome>,
) -> ContextAnalysisDropReason {
    match outcome {
        Some(ContextAnalysisOutcome::Dropped(reason)) => *reason,
        Some(ContextAnalysisOutcome::Analyzed(_)) | None => {
            ContextAnalysisDropReason::CorrelationDegraded
        }
        Some(_) => ContextAnalysisDropReason::Unsupported,
    }
}

fn account_durable_event_drop(counters: &ContextCounters, analysis_enabled: bool, event: RunEvent) {
    if !analysis_enabled {
        return;
    }
    match event {
        RunEvent::RequestContext(observation) => {
            let forward = observation.forward;
            counters.ensure_seen(forward);
            counters.dropped(
                forward,
                dropped_context_reason(observation.context.as_ref()),
            );
        }
        RunEvent::ContextAnalysis(forward, outcome, _shadow_body, analysis_permit) => {
            drop(analysis_permit);
            counters.ensure_seen(forward);
            counters.dropped(forward, dropped_context_reason(Some(&outcome)));
        }
        RunEvent::Transport(_)
        | RunEvent::Response(_, _, _)
        | RunEvent::TransportAdmissionFailed(_, _)
        | RunEvent::TransportFailure(_, _)
        | RunEvent::Compaction(_) => {}
    }
}

/// Context data handed from the provider recorder to the independent context worker.
///
/// Provider persistence has already returned all three durable identities before this value is
/// admitted. The context worker therefore never needs to re-open, or infer, provider state.
#[derive(Debug)]
pub(super) struct ContextIngestionJob {
    forward: ForwardId,
    provider_request_id: RequestId,
    attempt_id: AttemptId,
    inference_operation_id: OperationId,
    analysis: ContextAnalysisResult,
    shadow_body: Option<ShadowAnalysisBody>,
    analysis_permit: Option<AnalysisOutputPermit>,
    provider_input_tokens: Option<u64>,
    provider_usage_comparable: bool,
    correlation: CorrelationStatus,
}

/// Bounded permits for analyzed outcomes waiting to cross into context ingestion.
///
/// A full context-ingestion queue is auxiliary backpressure, not an analysis loss. The permit is
/// acquired from the detached analysis worker, so that worker waits for the single ingestion
/// worker to make room while forwarding remains independent. The number of retained analyzed
/// outcomes remains bounded by the existing hard queue capacity.
#[derive(Debug)]
struct AnalysisOutputSlots {
    available: Mutex<usize>,
    wake: Condvar,
}

#[derive(Debug)]
pub(super) struct AnalysisOutputPermit {
    slots: Arc<AnalysisOutputSlots>,
}

impl AnalysisOutputSlots {
    fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            available: Mutex::new(capacity.max(1)),
            wake: Condvar::new(),
        })
    }

    /// Waits off the forwarding/runtime path until one bounded outcome can be retained.
    fn acquire(self: &Arc<Self>) -> AnalysisOutputPermit {
        let mut available = self
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *available == 0 {
            available = self
                .wake
                .wait(available)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *available = available.saturating_sub(1);
        AnalysisOutputPermit {
            slots: Arc::clone(self),
        }
    }
}

impl Drop for AnalysisOutputPermit {
    fn drop(&mut self) {
        let mut available = self
            .slots
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *available = available.saturating_add(1);
        self.slots.wake.notify_one();
    }
}

/// Compact provider receipt retained while the detached context outcome is still in flight.
#[derive(Debug)]
pub(super) struct ContextReceipt {
    pub(super) forward: ForwardId,
    pub(super) provider_request_id: RequestId,
    pub(super) attempt_id: AttemptId,
    pub(super) inference_operation_id: OperationId,
    pub(super) provider_input_tokens: Option<u64>,
    pub(super) provider_usage_comparable: bool,
    pub(super) correlation: CorrelationStatus,
}

struct ContextAppendInput<'analysis> {
    snapshot_id: ContextSnapshotId,
    analysis: &'analysis ContextAnalysisResult,
    status: ContextAnalysisStatus,
}
#[derive(Clone, Copy)]
struct ContextCoverage {
    token_estimation_eligible: u64,
    token_estimation_observed: u64,
    semantic_detection_eligible: u64,
    semantic_detection_observed: u64,
    correlation_eligible: u64,
    correlation_correlated: u64,
}

pub(super) struct ContextAnalysisInput {
    pub(super) forward: ForwardId,
    pub(super) outcome: ContextAnalysisOutcome,
    pub(super) shadow_body: Option<ShadowAnalysisBody>,
    pub(super) analysis_permit: Option<AnalysisOutputPermit>,
}
struct ContextFailureInput<'context> {
    forward: ForwardId,
    context: Option<&'context ContextAnalysisOutcome>,
    analysis_permit: Option<AnalysisOutputPermit>,
}

struct EnqueueContextInput<'receipt> {
    receipt: &'receipt ContextReceipt,
    outcome: ContextAnalysisOutcome,
    shadow_body: Option<ShadowAnalysisBody>,
    analysis_permit: Option<AnalysisOutputPermit>,
}

struct ContextAppendBatchInput {
    snapshot_id: ContextSnapshotId,
    sequence: u32,
    blocks: Vec<tracepress_context::ContextBlockDraft>,
}
struct ContextFinalizationInput<'analysis> {
    forward: ForwardId,
    snapshot_id: ContextSnapshotId,
    started_at_us: u64,
    attempt_id: AttemptId,
    analysis: &'analysis ContextAnalysisResult,
    provider_input_tokens: Option<u64>,
    provider_usage_comparable: bool,
    correlation: CorrelationStatus,
    status: ContextAnalysisStatus,
    accepted_block_count: u64,
}
#[derive(Debug)]
struct RecordObservationInput {
    forward: ForwardId,
    semantic: SemanticRecord,
    correlation: CorrelationStatus,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DashboardOptions {
    pub(super) port: u16,
    pub(super) fixture: bool,
    pub(super) large_fixture: bool,
}

/// Accounting for context analyses rejected before context persistence.
#[derive(Debug, Default)]
pub(super) struct ContextCounters {
    observer_backpressure: AtomicU64,
    deferred_backlog_capacity: AtomicU64,
    resource_limit: AtomicU64,
    malformed: AtomicU64,
    pub(super) correlation_degraded: AtomicU64,
    unsupported: AtomicU64,
    cancelled: AtomicU64,
    pre_persistence_drop_backpressure: AtomicU64,
    pre_persistence_drop_deferred_backlog_capacity: AtomicU64,
    pre_persistence_drop_resource_limit: AtomicU64,
    pre_persistence_drop_malformed: AtomicU64,
    pre_persistence_drop_correlation: AtomicU64,
    pre_persistence_drop_unsupported: AtomicU64,
    pub(super) pre_persistence_drop_cancelled: AtomicU64,
    pending_drop_backpressure: AtomicU64,
    pending_drop_deferred_backlog_capacity: AtomicU64,
    pending_drop_resource_limit: AtomicU64,
    pending_drop_malformed: AtomicU64,
    pending_drop_correlation: AtomicU64,
    pending_drop_unsupported: AtomicU64,
    pub(super) pending_drop_cancelled: AtomicU64,
    pub(super) analysis_requests_seen: AtomicU64,
    pub(super) analysis_requests_complete: AtomicU64,
    pub(super) analysis_requests_partial: AtomicU64,
    pub(super) analysis_requests_dropped: AtomicU64,
    token_estimation_eligible: AtomicU64,
    token_estimation_observed: AtomicU64,
    semantic_detection_eligible: AtomicU64,
    semantic_detection_observed: AtomicU64,
    correlation_eligible: AtomicU64,
    correlation_correlated: AtomicU64,
    pub(super) lifecycle: Mutex<ContextLifecycle>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct AnalysisSequence(pub(super) u64);

impl From<ForwardId> for AnalysisSequence {
    fn from(forward: ForwardId) -> Self {
        Self(forward.get())
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingAnalysisEvidence {
    pub(super) sequence: AnalysisSequence,
    pub(super) provider_request_id: RequestId,
    pub(super) snapshot_id: Option<ContextSnapshotId>,
}

#[derive(Debug, Default)]
pub(super) struct ContextLifecycle {
    /// Analyses admitted by a sink but not yet terminally classified. Correlation admission
    /// bounds this set to the in-flight identity cap.
    pub(super) pending: BTreeSet<AnalysisSequence>,
    /// Durable provider/snapshot identities retained while an analysis is pending. This is
    /// bounded alongside `pending` and is used to reconcile a response lost after commit.
    pub(super) evidence: BTreeMap<AnalysisSequence, PendingAnalysisEvidence>,
    /// Terminal identities above the contiguous watermark, retained to reject reordered
    /// duplicates until the watermark catches up.
    pub(super) terminal_holes: BTreeSet<AnalysisSequence>,
    pub(super) terminal_through: Option<AnalysisSequence>,
}

impl ContextCounters {
    const fn counter(&self, reason: ContextAnalysisDropReason) -> &AtomicU64 {
        match reason {
            ContextAnalysisDropReason::ObserverBackpressure => &self.observer_backpressure,
            ContextAnalysisDropReason::DeferredBacklogCapacity => &self.deferred_backlog_capacity,
            ContextAnalysisDropReason::ResourceLimit => &self.resource_limit,
            ContextAnalysisDropReason::Malformed => &self.malformed,
            ContextAnalysisDropReason::CorrelationDegraded => &self.correlation_degraded,
            ContextAnalysisDropReason::Cancelled => &self.cancelled,
            _ => &self.unsupported,
        }
    }

    const fn pre_persistence_counter(&self, reason: ContextAnalysisDropReason) -> &AtomicU64 {
        match reason {
            ContextAnalysisDropReason::ObserverBackpressure => {
                &self.pre_persistence_drop_backpressure
            }
            ContextAnalysisDropReason::DeferredBacklogCapacity => {
                &self.pre_persistence_drop_deferred_backlog_capacity
            }
            ContextAnalysisDropReason::ResourceLimit => &self.pre_persistence_drop_resource_limit,
            ContextAnalysisDropReason::Malformed => &self.pre_persistence_drop_malformed,
            ContextAnalysisDropReason::CorrelationDegraded => {
                &self.pre_persistence_drop_correlation
            }
            ContextAnalysisDropReason::Cancelled => &self.pre_persistence_drop_cancelled,
            _ => &self.pre_persistence_drop_unsupported,
        }
    }

    const fn pending_drop_counter(&self, reason: ContextAnalysisDropReason) -> &AtomicU64 {
        match reason {
            ContextAnalysisDropReason::ObserverBackpressure => &self.pending_drop_backpressure,
            ContextAnalysisDropReason::DeferredBacklogCapacity => {
                &self.pending_drop_deferred_backlog_capacity
            }
            ContextAnalysisDropReason::ResourceLimit => &self.pending_drop_resource_limit,
            ContextAnalysisDropReason::Malformed => &self.pending_drop_malformed,
            ContextAnalysisDropReason::CorrelationDegraded => &self.pending_drop_correlation,
            ContextAnalysisDropReason::Cancelled => &self.pending_drop_cancelled,
            _ => &self.pending_drop_unsupported,
        }
    }
    /// Admits one eligible Phase 3 request exactly once.
    fn ensure_seen(&self, forward: ForwardId) {
        self.ensure_seen_sequence(AnalysisSequence::from(forward));
    }

    pub(super) fn ensure_seen_sequence(&self, sequence: AnalysisSequence) {
        let inserted = {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let terminal_through = lifecycle
                .terminal_through
                .is_some_and(|terminal_through| sequence <= terminal_through);
            if lifecycle.pending.contains(&sequence)
                || terminal_through
                || lifecycle.terminal_holes.contains(&sequence)
            {
                false
            } else {
                lifecycle.pending.insert(sequence)
            }
        };
        if inserted {
            let _counted = self.analysis_requests_seen.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Remembers the durable provider identity needed to reconcile a lost context response.
    fn remember_provider_request(&self, forward: ForwardId, provider_request_id: RequestId) {
        let sequence = AnalysisSequence::from(forward);
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if lifecycle.pending.contains(&sequence)
            && (lifecycle.evidence.contains_key(&sequence)
                || lifecycle.evidence.len() < IN_FLIGHT_FORWARDS)
        {
            let _ = lifecycle
                .evidence
                .entry(sequence)
                .and_modify(|evidence| evidence.provider_request_id = provider_request_id)
                .or_insert(PendingAnalysisEvidence {
                    sequence,
                    provider_request_id,
                    snapshot_id: None,
                });
        }
    }

    /// Associates the daemon snapshot allocated by `BeginContextAnalysis` with its provider row.
    fn remember_snapshot(&self, forward: ForwardId, snapshot_id: ContextSnapshotId) {
        let sequence = AnalysisSequence::from(forward);
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(evidence) = lifecycle.evidence.get_mut(&sequence) {
            evidence.snapshot_id = Some(snapshot_id);
        }
    }

    pub(super) fn pending_evidence(&self) -> Vec<PendingAnalysisEvidence> {
        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lifecycle.evidence.values().copied().collect()
    }

    fn mark_terminal(&self, forward: ForwardId) -> bool {
        self.mark_terminal_sequence(AnalysisSequence::from(forward))
    }

    fn mark_terminal_sequence(&self, sequence: AnalysisSequence) -> bool {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let terminal = if lifecycle.pending.remove(&sequence) {
            let _ = lifecycle.evidence.remove(&sequence);
            Self::record_terminal_locked(&mut lifecycle, sequence);
            true
        } else {
            false
        };
        drop(lifecycle);
        terminal
    }

    fn record_terminal_locked(lifecycle: &mut ContextLifecycle, sequence: AnalysisSequence) {
        let _ = lifecycle.evidence.remove(&sequence);
        if lifecycle
            .terminal_through
            .is_some_and(|terminal_through| sequence <= terminal_through)
        {
            return;
        }
        let _inserted = lifecycle.terminal_holes.insert(sequence);
        loop {
            let next = lifecycle
                .terminal_through
                .map_or(0, |terminal_through| terminal_through.0.saturating_add(1));
            let Some(smallest) = lifecycle.terminal_holes.first().copied() else {
                break;
            };
            if smallest.0 != next {
                break;
            }
            let _folded = lifecycle.terminal_holes.pop_first();
            lifecycle.terminal_through = Some(smallest);
        }
        while lifecycle.terminal_holes.len() > TERMINAL_ANALYSIS_IDENTITIES {
            let _forgotten = lifecycle.terminal_holes.pop_first();
        }
    }

    fn complete(&self, forward: ForwardId) {
        self.complete_sequence(AnalysisSequence::from(forward));
    }

    pub(super) fn complete_sequence(&self, sequence: AnalysisSequence) {
        if self.mark_terminal_sequence(sequence) {
            let _counted = self
                .analysis_requests_complete
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn partial(&self, forward: ForwardId) {
        self.partial_sequence(AnalysisSequence::from(forward));
    }

    pub(super) fn partial_sequence(&self, sequence: AnalysisSequence) {
        if self.mark_terminal_sequence(sequence) {
            let _counted = self
                .analysis_requests_partial
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn partial_with_drop_reason(&self, forward: ForwardId, reason: ContextAnalysisDropReason) {
        if self.mark_terminal(forward) {
            let _counted = self
                .analysis_requests_partial
                .fetch_add(1, Ordering::Relaxed);
            let _counted = self.counter(reason).fetch_add(1, Ordering::Relaxed);
        }
    }

    fn dropped(&self, forward: ForwardId, reason: ContextAnalysisDropReason) {
        if self.mark_terminal(forward) {
            let _counted = self.counter(reason).fetch_add(1, Ordering::Relaxed);
            let _counted = self
                .pre_persistence_counter(reason)
                .fetch_add(1, Ordering::Relaxed);
            let _counted = self
                .pending_drop_counter(reason)
                .fetch_add(1, Ordering::Relaxed);
            let _counted = self
                .analysis_requests_dropped
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Atomically terminal-accounts every admitted request that has not reached a durable
    /// terminal snapshot. This is used before worker cancellation on a forced drain timeout.
    pub(super) fn drop_pending(&self, reason: ContextAnalysisDropReason) {
        let count = {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let pending = std::mem::take(&mut lifecycle.pending);
            let count = u64::try_from(pending.len()).unwrap_or(u64::MAX);
            for sequence in pending {
                Self::record_terminal_locked(&mut lifecycle, sequence);
            }
            drop(lifecycle);
            count
        };
        if count == 0 {
            return;
        }
        let _counted = self.counter(reason).fetch_add(count, Ordering::Relaxed);
        let _counted = self
            .pre_persistence_counter(reason)
            .fetch_add(count, Ordering::Relaxed);
        let _counted = self
            .pending_drop_counter(reason)
            .fetch_add(count, Ordering::Relaxed);
        let _counted = self
            .analysis_requests_dropped
            .fetch_add(count, Ordering::Relaxed);
    }

    fn observe_coverage(&self, coverage: ContextCoverage) {
        let ContextCoverage {
            token_estimation_eligible,
            token_estimation_observed,
            semantic_detection_eligible,
            semantic_detection_observed,
            correlation_eligible,
            correlation_correlated,
        } = coverage;
        let _counted = self
            .token_estimation_eligible
            .fetch_add(token_estimation_eligible, Ordering::Relaxed);
        let _counted = self
            .token_estimation_observed
            .fetch_add(token_estimation_observed, Ordering::Relaxed);
        let _counted = self
            .semantic_detection_eligible
            .fetch_add(semantic_detection_eligible, Ordering::Relaxed);
        let _counted = self
            .semantic_detection_observed
            .fetch_add(semantic_detection_observed, Ordering::Relaxed);
        let _counted = self
            .correlation_eligible
            .fetch_add(correlation_eligible, Ordering::Relaxed);
        let _counted = self
            .correlation_correlated
            .fetch_add(correlation_correlated, Ordering::Relaxed);
    }

    fn take_pending_drop_count(&self, reason: ContextAnalysisDropReason) -> u64 {
        self.pending_drop_counter(reason).swap(0, Ordering::Relaxed)
    }

    pub(super) fn report(&self) -> String {
        let seen = self.analysis_requests_seen.load(Ordering::Relaxed);
        let complete = self.analysis_requests_complete.load(Ordering::Relaxed);
        let partial = self.analysis_requests_partial.load(Ordering::Relaxed);
        let dropped = self.analysis_requests_dropped.load(Ordering::Relaxed);
        let coverage = |observed: u64, eligible: u64| {
            if eligible == 0 {
                "unknown".to_owned()
            } else {
                format!("{observed}/{eligible}")
            }
        };
        let complete_coverage = coverage(complete, seen);
        let partial_coverage = coverage(partial, seen);
        let dropped_coverage = coverage(dropped, seen);
        let analysis_coverage = coverage(complete, seen);
        let token_coverage = coverage(
            self.token_estimation_observed.load(Ordering::Relaxed),
            self.token_estimation_eligible.load(Ordering::Relaxed),
        );
        let semantic_coverage = coverage(
            self.semantic_detection_observed.load(Ordering::Relaxed),
            self.semantic_detection_eligible.load(Ordering::Relaxed),
        );
        let correlation_eligible = self.correlation_eligible.load(Ordering::Relaxed);
        let correlation_correlated = self.correlation_correlated.load(Ordering::Relaxed);
        let correlation_coverage = coverage(correlation_correlated, correlation_eligible);
        format!(
            "context_observer_backpressure_total={}\ncontext_deferred_backlog_capacity_total={}\ncontext_resource_limit_total={}\ncontext_malformed_total={}\ncontext_correlation_degraded_total={}\ncontext_unsupported_total={}\ncontext_cancelled_total={}\nanalysis_requests_seen={seen}\nanalysis_requests_complete={complete}\nanalysis_requests_partial={partial}\nanalysis_requests_dropped={dropped}\nanalysis_drop_backpressure={}\nanalysis_drop_deferred_backlog_capacity={}\nanalysis_drop_resource_limit={}\nanalysis_drop_malformed={}\nanalysis_drop_correlation={}\nanalysis_drop_unsupported={}\nanalysis_drop_cancelled={}\ncorrelation_eligible={correlation_eligible}\ncorrelation_correlated={correlation_correlated}\nanalysis_coverage={analysis_coverage}\ncorrelation_coverage={correlation_coverage}\nanalysis_coverage_complete={complete_coverage}\nanalysis_coverage_partial={partial_coverage}\nanalysis_coverage_dropped={dropped_coverage}\ntoken_estimation_coverage={token_coverage}\nsemantic_detection_coverage={semantic_coverage}",
            self.observer_backpressure.load(Ordering::Relaxed),
            self.deferred_backlog_capacity.load(Ordering::Relaxed),
            self.resource_limit.load(Ordering::Relaxed),
            self.malformed.load(Ordering::Relaxed),
            self.correlation_degraded.load(Ordering::Relaxed),
            self.unsupported.load(Ordering::Relaxed),
            self.cancelled.load(Ordering::Relaxed),
            self.pre_persistence_drop_backpressure
                .load(Ordering::Relaxed),
            self.pre_persistence_drop_deferred_backlog_capacity
                .load(Ordering::Relaxed),
            self.pre_persistence_drop_resource_limit
                .load(Ordering::Relaxed),
            self.pre_persistence_drop_malformed.load(Ordering::Relaxed),
            self.pre_persistence_drop_correlation
                .load(Ordering::Relaxed),
            self.pre_persistence_drop_unsupported
                .load(Ordering::Relaxed),
            self.pre_persistence_drop_cancelled.load(Ordering::Relaxed),
        )
    }
}

/// Builds a provider observation request that fits the bounded control protocol.
///
/// Provider usage is retained exactly up to the durable `SQLite` bound, but `RawProviderUsage` is
/// serialized as a JSON byte array on IPC. A large, otherwise valid usage object can therefore
/// exceed the smaller control-body bound. The raw object is optional evidence: normalized usage
/// remains provider truth and is kept in the observation while only oversized metadata is removed
/// at this daemon boundary. No request or response wire body passes through this helper.
pub(super) fn bounded_record_provider_observation_request(
    session_id: SessionId,
    parent_operation_id: OperationId,
    mut observation: ObservationRecord,
) -> ControlRequest {
    let ids = UuidV7Generator::new();
    let request_id = RequestId::generate(&ids);
    let maximum_body = MaxRequestBodyBytes::new(BODY_BYTES).ok();
    let fits = |candidate: &ObservationRecord| {
        let request = ControlRequest::RecordProviderObservation {
            session_id,
            parent_operation_id,
            observation: Box::new(candidate.clone()),
        };
        let Ok(body) = serde_json::to_vec(&request) else {
            return false;
        };
        let body_fits = u64::try_from(body.len()).is_ok_and(|length| length <= BODY_BYTES);
        let Some(maximum_body) = maximum_body else {
            return false;
        };
        let Ok(ipc_request) = IpcRequest::new(request_id, body, maximum_body) else {
            return false;
        };
        let frame_fits = serde_json::to_vec(&ipc_request).is_ok_and(|frame| {
            u64::try_from(frame.len()).is_ok_and(|length| length <= FRAME_BYTES)
        });
        body_fits && frame_fits
    };

    if !fits(&observation)
        && let Some(response) = observation.response.as_mut()
    {
        response.raw_usage = None;
    }
    if !fits(&observation)
        && let Some(response) = observation.response.as_mut()
    {
        response.provider_response_id = None;
        response.model = None;
        response.incomplete_reason = None;
        response.error_code = None;
    }
    if !fits(&observation) {
        observation.request.model = None;
        observation.request.reasoning_effort = None;
        observation.request.verbosity = None;
        observation.request.truncation = None;
    }
    if !fits(&observation) {
        observation.response = None;
        observation.transport_error = None;
    }

    ControlRequest::RecordProviderObservation {
        session_id,
        parent_operation_id,
        observation: Box::new(observation),
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
pub(super) struct ForwardState {
    request: Option<PendingObservation>,
    response: Option<ResponseObservation>,
    context: Option<ContextAnalysisOutcome>,
    shadow_body: Option<ShadowAnalysisBody>,
    context_analysis_permit: Option<AnalysisOutputPermit>,
    status_code: Option<u16>,
    transport_failure: Option<TransportFailure>,
    /// Transport admission failed before status could be joined to this forward.
    transport_admission_failure: Option<CorrelationDegradation>,
    transport: bool,
    settled: bool,
}

impl ForwardState {
    /// Returns whether the forwarding half has reached a terminal state that can be joined to
    /// semantic evidence.
    const fn transport_is_terminal(&self) -> bool {
        self.transport
            || self.transport_failure.is_some()
            || self.transport_admission_failure.is_some()
    }

    /// Returns whether this forward has all evidence required for one durable provider
    /// observation.
    ///
    /// Context analysis is detached from forwarding. A response therefore settles the bounded
    /// correlation identity as soon as provider evidence is complete; the compact receipt is
    /// retained separately until the deferred context outcome arrives.
    const fn is_ready_to_settle(&self) -> bool {
        self.request.is_some()
            && (self.response.is_some() || self.transport_failure.is_some())
            && self.transport_is_terminal()
    }

    /// Takes the evidence observed for this forward so far.
    ///
    /// A response without its request half carries no logical request, so it is dropped in
    /// favour of whatever transport evidence the forward has, and the forward reports that its
    /// correlation is missing rather than presenting transport evidence as a whole exchange.
    fn evidence(&mut self, forward: ForwardId) -> ForwardEvidence {
        let status_code = self.status_code;
        let response = self.response.take();
        let context = self.context.take();
        let shadow_body = self.shadow_body.take();
        let context_analysis_permit = self.context_analysis_permit.take();
        let transport_failure = self.transport_failure.take();
        let orphaned_semantic =
            self.request.is_none() && (response.is_some() || transport_failure.is_some());
        ForwardEvidence {
            forward,
            semantic: self.request.take().map(|pending| SemanticRecord {
                pending,
                response,
                context,
                shadow_body,
                analysis_permit: context_analysis_permit,
                status_code,
                transport_failure,
            }),
            orphaned_semantic,
            transport: self.transport,
            transport_admission_failure: self.transport_admission_failure,
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
    transport_admission_failure: Option<CorrelationDegradation>,
}

/// Semantic evidence of one forward, with the transport evidence its forwarding half observed.
#[derive(Debug)]
struct SemanticRecord {
    pending: PendingObservation,
    response: Option<ResponseObservation>,
    context: Option<ContextAnalysisOutcome>,
    shadow_body: Option<ShadowAnalysisBody>,
    analysis_permit: Option<AnalysisOutputPermit>,
    status_code: Option<u16>,
    transport_failure: Option<TransportFailure>,
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
pub(super) struct RunRecorder {
    pub(super) config: Config,
    pub(super) session_id: SessionId,
    pub(super) parent_operation_id: OperationId,
    pub(super) context_sender: tokio::sync::mpsc::Sender<ContextIngestionJob>,
    pub(super) context_counters: Arc<ContextCounters>,
    pub(super) forwards: BTreeMap<ForwardId, ForwardState>,
    pub(super) analysis_enabled: bool,
    /// Terminal response halves retained briefly when a request parser finishes after eviction.
    pub(super) retired_orphans: BTreeMap<ForwardId, ForwardState>,
    /// Bounded receipts waiting for a detached context outcome.
    pub(super) pending_context: BTreeMap<ForwardId, ContextReceipt>,
    /// Identities dropped from correlation state that the watermark does not cover yet.
    ///
    /// Bounded by [`RETIRED_IDENTITIES`]; its contiguous prefix is folded into the watermark.
    pub(super) retired: BTreeSet<ForwardId>,
    /// Watermark at or below which every identity has already been retired, once one exists.
    ///
    /// Identities are monotonic per proxy, so a contiguous run of retired identities collapses
    /// into one watermark: refusing everything at or below it refuses only forwards this
    /// recorder has already accounted for.
    pub(super) retired_through: Option<ForwardId>,
    /// Correlation losses observed so far, shared with the run that reports them.
    pub(super) counters: Arc<CorrelationCounters>,
}

impl RunRecorder {
    async fn run(
        mut self,
        mut events: tokio::sync::mpsc::Receiver<RunEvent>,
    ) -> Result<(), String> {
        let mut first_error = None;
        while let Some(event) = events.recv().await {
            match event {
                RunEvent::Transport(metadata) => {
                    if let Err(error) = self.transport(metadata).await {
                        let _first = first_error.get_or_insert(error);
                    }
                }
                RunEvent::RequestContext(observation) => {
                    self.request_context(*observation).await;
                }
                RunEvent::ContextAnalysis(forward, outcome, shadow_body, analysis_permit) => {
                    self.context_analysis(ContextAnalysisInput {
                        forward,
                        outcome,
                        shadow_body,
                        analysis_permit,
                    })
                    .await;
                }
                RunEvent::Response(forward, observation, correlation) => {
                    self.response(forward, *observation, correlation).await;
                }
                RunEvent::TransportAdmissionFailed(forward, reason) => {
                    self.transport_admission_failed(forward, reason).await;
                }
                RunEvent::TransportFailure(forward, failure) => {
                    self.transport_failure(forward, failure).await;
                }
                RunEvent::Compaction(observation) => {
                    if let Err(error) = self.compaction(*observation).await {
                        let _first = first_error.get_or_insert(error);
                    }
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
                self.drop_unpaired_context(forward, &mut state);
            }
            // The run ending is not a bound degradation: the forward keeps the evidence it has.
            {
                let record = self.record(state.evidence(forward), CorrelationStatus::Correlated);
                record.await;
            }
        }
        // Receipts with no detached outcome have reached a terminal provider record but can no
        // longer be attributed. Account every one explicitly instead of silently dropping the map.
        while let Some((forward, _receipt)) = self.pending_context.pop_first() {
            self.context_counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
        }
        while let Some((forward, mut state)) = self.retired_orphans.pop_first() {
            self.drop_unpaired_context(forward, &mut state);
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Records an unobserved route's forward, or joins the status onto a Responses forward.
    async fn transport(&mut self, metadata: ForwardMetadata) -> Result<(), String> {
        if !matches!(metadata.route, InboundRoute::Responses) {
            // Every route without semantic observation keeps its transport-only inference.
            return match self.record_forward().await? {
                ControlResponse::Ok { .. } => Ok(()),
                ControlResponse::Error { message } => Err(message),
                ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
                    Err("daemon returned an unexpected context response".to_owned())
                }
            };
        }
        let mut evidence = Some({
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
        });
        if let Some(evidence) = evidence.take() {
            let record = self.record(evidence, CorrelationStatus::Correlated);
            record.await;
        }
        Ok(())
    }

    /// Records compaction as a dedicated causal operation without invoking Responses parsing.
    async fn compaction(&self, observation: CompactionObservation) -> Result<(), String> {
        let ended_at = current_timestamp()?;
        let mut request = RequestObservation::transport_only(
            ProviderRequestKind::Compaction {
                protocol: tracepress_provider::CompactionProtocol::DedicatedEndpointLegacy,
                trigger: tracepress_provider::CompactionTrigger::Unknown,
            },
            observation.request_bytes,
            observation.request_bytes,
            observation.content_encoding,
            observation.transport,
            observation.endpoint_profile_version,
        );
        request.wire_sha256 = Some(observation.wire_sha256.to_vec().into_boxed_slice());
        let mut record =
            ObservationRecord::new(observation.request_bytes, request, ended_at.clone())
                .with_transport_metrics(observation.response_bytes, observation.duration_us)
                .with_outcome(match observation.outcome {
                    tracepress_proxy::CompactionOutcome::Completed => {
                        ProviderObservationOutcome::Completed
                    }
                    tracepress_proxy::CompactionOutcome::Failed => {
                        ProviderObservationOutcome::Failed
                    }
                    tracepress_proxy::CompactionOutcome::Incomplete => {
                        ProviderObservationOutcome::Incomplete
                    }
                    tracepress_proxy::CompactionOutcome::Cancelled => {
                        ProviderObservationOutcome::Cancelled
                    }
                    tracepress_proxy::CompactionOutcome::Disconnected => {
                        ProviderObservationOutcome::Disconnected
                    }
                    _ => ProviderObservationOutcome::Incomplete,
                })
                .with_ended_at(ended_at);
        if let Some(status) = observation
            .status_code
            .and_then(|status| HttpStatusCode::new(status).ok())
        {
            record = record.with_status_code(status);
        }
        if let Some(failure) = observation.transport_failure {
            record = record.with_transport_error(failure.label());
        }
        let response = control(
            &self.config,
            bounded_record_provider_observation_request(
                self.session_id,
                self.parent_operation_id,
                record,
            ),
        )
        .await;
        match response {
            Ok(ControlResponse::Ok { .. }) => Ok(()),
            Ok(ControlResponse::Error { message }) => Err(message),
            Ok(ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. }) => {
                Err("daemon returned an unexpected context response".to_owned())
            }
            Err(error) => Err(error),
        }
    }

    /// Buffers the Phase 2 request half before dispatch. Provider evidence can settle without
    /// waiting for the detached Phase 3 outcome; that outcome is joined through `pending_context`.
    async fn request_context(&mut self, observation: RequestContextObservation) {
        let analysis_enabled = self.analysis_enabled;
        let forward = observation.forward;
        if analysis_enabled {
            self.context_counters.ensure_seen(forward);
        }
        let Ok(started_at) = current_timestamp() else {
            if analysis_enabled {
                self.context_counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            }
            return;
        };
        if let Some(mut state) = self.retired_orphans.remove(&forward) {
            if state.settled {
                if analysis_enabled {
                    self.context_counters
                        .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
                }
                return;
            }
            state.request = Some(PendingObservation {
                request: observation.observation,
                started_at,
            });
            if let Some(context) = observation.context {
                state.context = Some(context);
            }
            if !state.is_ready_to_settle() {
                return;
            }
            state.settled = true;
            {
                let record = self.record(state.evidence(forward), CorrelationStatus::Correlated);
                record.await;
            }
            return;
        }
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            if analysis_enabled {
                self.context_counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            }
            return;
        };
        if state.settled {
            if analysis_enabled {
                self.context_counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            }
            return;
        }
        state.request = Some(PendingObservation {
            request: observation.observation,
            started_at,
        });
        if let Some(context) = observation.context {
            state.context = Some(context);
        }
        if !state.is_ready_to_settle() {
            return;
        }
        state.settled = true;
        let mut evidence = Some(state.evidence(forward));
        if let Some(evidence) = evidence.take() {
            let record = self.record(evidence, CorrelationStatus::Correlated);
            record.await;
        }
    }

    /// Attaches the detached Phase 3 outcome after the provider record has crossed dispatch.
    pub(super) async fn context_analysis(&mut self, input: ContextAnalysisInput) {
        let ContextAnalysisInput {
            forward,
            outcome,
            shadow_body,
            analysis_permit,
        } = input;
        if self.analysis_enabled {
            self.context_counters.ensure_seen(forward);
        }
        // The detached analyzer can finish before the request parser publishes its metadata.
        // A durable provider receipt takes precedence over correlation tombstones: the forward
        // may have been retired after its provider record settled while its analysis was still
        // pending. Consuming that receipt is the only way to preserve the one-outcome partition.
        if let Some(receipt) = self.pending_context.remove(&forward) {
            self.enqueue_context(EnqueueContextInput {
                receipt: &receipt,
                outcome,
                shadow_body,
                analysis_permit,
            });
            return;
        }
        // Admit the identity here so an early analysis outcome is retained and can be joined by
        // the later request/response halves instead of being misclassified as correlation loss.
        // A retired identity is already terminal and must not be re-admitted.
        if !self.forwards.contains_key(&forward) {
            if self.is_retired(forward) {
                drop(analysis_permit);
                self.context_counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
                return;
            }
            self.admit(forward).await;
        }
        let Some(state) = self.forwards.get_mut(&forward) else {
            self.context_counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return;
        };
        if state.settled {
            drop(analysis_permit);
            self.context_counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return;
        }
        state.context = Some(outcome);
        state.shadow_body = shadow_body;
        state.context_analysis_permit = analysis_permit;
        if !state.is_ready_to_settle() {
            return;
        }
        state.settled = true;
        let mut evidence = Some(state.evidence(forward));
        if let Some(evidence) = evidence.take() {
            let record = self.record(evidence, CorrelationStatus::Correlated);
            record.await;
        }
    }

    /// Marks a forward whose transport half was refused by bounded admission.
    async fn transport_admission_failed(
        &mut self,
        forward: ForwardId,
        reason: CorrelationDegradation,
    ) {
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        state.transport_admission_failure = Some(reason);
        if !state.is_ready_to_settle() {
            return;
        }
        state.settled = true;
        let evidence = state.evidence(forward);
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Buffers the terminal semantic response half, settling the forward once paired.
    ///
    /// The forwarding half reports transport metadata from a detached dispatcher, so it may
    /// still be in flight here; successful responses wait for that status before persistence.
    #[allow(
        clippy::too_many_arguments,
        reason = "response event fields are kept explicit at the recorder boundary"
    )]
    async fn response(
        &mut self,
        forward: ForwardId,
        observation: ResponseObservation,
        correlation: CorrelationStatus,
    ) {
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        if let CorrelationStatus::Degraded(reason) = correlation {
            state.transport_admission_failure = Some(reason);
        }
        state.response = Some(observation);
        if state.request.is_none()
            || (!state.transport && state.transport_admission_failure.is_none())
        {
            return;
        }
        state.settled = true;
        let evidence = state.evidence(forward);
        self.record(evidence, correlation).await;
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
        let mut evidence = Some(state.evidence(forward));
        if let Some(evidence) = evidence.take() {
            let record = self.record(evidence, CorrelationStatus::Correlated);
            record.await;
        }
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
                self.drop_unpaired_context(oldest, &mut state);
            }
            if state.request.is_none()
                && (state.response.is_some() || state.transport_failure.is_some())
            {
                // Terminal semantic evidence without a request cannot become a provider row.
                // Persist its transport-only operation, then retain a settled tombstone so the
                // late parser half is consumed rather than re-admitting this identity.
                let mut evidence = Some(state.evidence(oldest));
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
                if let Some(evidence) = evidence.take() {
                    let record = self.record(
                        evidence,
                        CorrelationStatus::Degraded(CorrelationDegradation::InFlightLimit),
                    );
                    record.await;
                }
                continue;
            }
            let mut evidence = Some(state.evidence(oldest));
            // The bound, not the exchange, ended this forward's correlation: it is recorded
            // with the evidence it has and marked degraded, never as a whole exchange.
            if let Some(evidence) = evidence.take() {
                let record = self.record(
                    evidence,
                    CorrelationStatus::Degraded(CorrelationDegradation::InFlightLimit),
                );
                record.await;
            }
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
    async fn record(&mut self, evidence: ForwardEvidence, fallback: CorrelationStatus) {
        let ForwardEvidence {
            forward,
            semantic,
            orphaned_semantic,
            transport,
            transport_admission_failure,
        } = evidence;
        let correlation = if matches!(fallback, CorrelationStatus::Degraded(_)) {
            fallback
        } else {
            transport_admission_failure.map_or(fallback, CorrelationStatus::Degraded)
        };
        let admission_was_counted = match (transport_admission_failure, correlation) {
            (Some(admission), CorrelationStatus::Degraded(reason)) => admission == reason,
            _ => false,
        };
        if let CorrelationStatus::Degraded(reason) = correlation
            && !admission_was_counted
        {
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
        let analysis_enabled = self.analysis_enabled;
        let RecordObservationInput {
            forward,
            semantic,
            correlation,
        } = input;
        let Ok(ended_at) = current_timestamp() else {
            if analysis_enabled {
                self.context_counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            }
            return false;
        };
        let Some((
            observation,
            context,
            shadow_body,
            analysis_permit,
            provider_input,
            provider_usage_comparable,
        )) = observation_record(semantic, ended_at, correlation)
        else {
            if analysis_enabled {
                self.context_counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            }
            return false;
        };
        let response = control(
            &self.config,
            bounded_record_provider_observation_request(
                self.session_id,
                self.parent_operation_id,
                observation,
            ),
        )
        .await;
        let Ok(ControlResponse::Ok {
            provider_request_id: Some(provider_request_id),
            attempt_id: Some(attempt_id),
            inference_operation_id: Some(inference_operation_id),
            ..
        }) = response
        else {
            self.account_context_failure(ContextFailureInput {
                forward,
                context: context.as_ref(),
                analysis_permit,
            });
            return false;
        };
        if analysis_enabled {
            self.context_counters
                .remember_provider_request(forward, provider_request_id);
        }
        let receipt = ContextReceipt {
            forward,
            provider_request_id,
            attempt_id,
            inference_operation_id,
            provider_input_tokens: provider_input,
            provider_usage_comparable,
            correlation,
        };
        match context {
            Some(outcome) => self.enqueue_context(EnqueueContextInput {
                receipt: &receipt,
                outcome,
                shadow_body,
                analysis_permit,
            }),
            None if self.analysis_enabled => {
                if self.pending_context.len() >= IN_FLIGHT_FORWARDS
                    && let Some((evicted, _receipt)) = self.pending_context.pop_first()
                {
                    self.context_counters
                        .dropped(evicted, ContextAnalysisDropReason::CorrelationDegraded);
                }
                if self.pending_context.insert(forward, receipt).is_some() {
                    self.context_counters
                        .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
                }
                drop(analysis_permit);
            }
            None => {
                // Phase 2 remains durable when Phase 3 is disabled; without an analysis outcome
                // there is nothing to retain or flush as a context drop.
                drop(analysis_permit);
            }
        }
        true
    }
    fn account_context_failure(&self, input: ContextFailureInput<'_>) {
        let ContextFailureInput {
            forward,
            context,
            analysis_permit,
        } = input;
        drop(analysis_permit);
        if !self.analysis_enabled {
            return;
        }
        match context {
            Some(ContextAnalysisOutcome::Dropped(reason)) => {
                self.context_counters.dropped(forward, *reason);
            }
            Some(ContextAnalysisOutcome::Analyzed(_)) | None => self
                .context_counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded),
            _ => self
                .context_counters
                .dropped(forward, ContextAnalysisDropReason::Unsupported),
        }
    }

    fn enqueue_context(&self, input: EnqueueContextInput<'_>) {
        let EnqueueContextInput {
            receipt,
            outcome,
            shadow_body,
            analysis_permit,
        } = input;
        match outcome {
            ContextAnalysisOutcome::Analyzed(analysis) => {
                let job = ContextIngestionJob {
                    forward: receipt.forward,
                    provider_request_id: receipt.provider_request_id,
                    attempt_id: receipt.attempt_id,
                    inference_operation_id: receipt.inference_operation_id,
                    analysis,
                    shadow_body,
                    analysis_permit,
                    provider_input_tokens: receipt.provider_input_tokens,
                    provider_usage_comparable: receipt.provider_usage_comparable,
                    correlation: receipt.correlation,
                };
                if self.context_sender.try_send(job).is_err() {
                    self.context_counters.dropped(
                        receipt.forward,
                        ContextAnalysisDropReason::ObserverBackpressure,
                    );
                }
            }
            ContextAnalysisOutcome::Dropped(reason) => {
                drop(analysis_permit);
                self.context_counters.dropped(receipt.forward, reason);
            }
            _ => {
                drop(analysis_permit);
                self.context_counters
                    .dropped(receipt.forward, ContextAnalysisDropReason::Unsupported);
            }
        }
    }

    fn drop_unpaired_context(&self, forward: ForwardId, state: &mut ForwardState) {
        let Some(outcome) = state.context.take() else {
            self.context_counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return;
        };
        let reason = match outcome {
            ContextAnalysisOutcome::Analyzed(_analysis) => {
                ContextAnalysisDropReason::CorrelationDegraded
            }
            ContextAnalysisOutcome::Dropped(reason) => reason,
            _ => ContextAnalysisDropReason::Unsupported,
        };
        self.context_counters.dropped(forward, reason);
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

/// Session identities a recorder attributes one run's evidence to.
pub(super) struct RunRecording {
    pub(super) config: Config,
    pub(super) session_id: SessionId,
    pub(super) parent_operation_id: OperationId,
    pub(super) context_queue_items: usize,
    pub(super) context_analysis_limits: ContextAnalysisLimits,
    pub(super) analysis_enabled: bool,
    pub(super) shadow_enabled: bool,
    pub(super) shadow_experiment_id: String,
    pub(super) previous_context: Option<PreviousContextSnapshot>,
}
/// The recorder of one run: its proxy, its workers, and the counters it publishes.
pub(super) struct SpawnedRecorder {
    pub(super) proxy: TransparentProxy,
    pub(super) recorder_task: tokio::task::JoinHandle<Result<(), String>>,
    pub(super) transport_task: tokio::task::JoinHandle<()>,
    pub(super) context_task: tokio::task::JoinHandle<Result<(), String>>,
    pub(super) shadow_task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    /// Correlation accounting the run reports, readable whether or not the workers finished.
    pub(super) counters: Arc<CorrelationCounters>,
    /// Context analyses rejected by the bounded context queue.
    pub(super) context_counters: Arc<ContextCounters>,
    pub(super) active_counters: Arc<ActiveCompressionCounters>,
}

/// Installs both auxiliary sinks and starts the provider and context workers.
#[allow(
    clippy::too_many_lines,
    reason = "worker ownership and channel closure ordering stay together"
)]
pub(super) fn spawn_recorder(recording: RunRecording, proxy: TransparentProxy) -> SpawnedRecorder {
    let RunRecording {
        config,
        session_id,
        parent_operation_id,
        context_queue_items,
        context_analysis_limits,
        analysis_enabled,
        shadow_enabled,
        shadow_experiment_id,
        previous_context,
    } = recording;
    let (sender, receiver) = tokio::sync::mpsc::channel(RECORDER_QUEUE_ITEMS);
    let (transport_sender, transport_receiver) =
        tokio::sync::mpsc::channel(TRANSPORT_DISPATCH_QUEUE_ITEMS);
    let context_queue_items = context_ingestion_queue_capacity(context_queue_items);
    let (context_sender, context_receiver) = tokio::sync::mpsc::channel(context_queue_items);
    let (shadow_sender, shadow_receiver) = tokio::sync::mpsc::channel(SHADOW_QUEUE_ITEMS);
    let counters = Arc::new(CorrelationCounters::default());
    let context_counters = Arc::new(ContextCounters::default());
    let active_counters = Arc::new(ActiveCompressionCounters::default());
    let shadow_counters = Arc::new(ShadowCounters::default());
    let shadow_budget = Arc::new(ShadowByteBudget::new(SHADOW_QUEUE_BYTES));
    let analysis_slots = AnalysisOutputSlots::new(context_queue_items);
    let shadow_task = shadow_enabled.then(|| {
        tokio::spawn(
            ShadowCompressionWorker {
                config: config.clone(),
                experiment_id: shadow_experiment_id,
                limits: CompressionLimits::default(),
                context_limits: context_analysis_limits,
                counters: Arc::clone(&shadow_counters),
            }
            .run(shadow_receiver),
        )
    });
    let shadow_sender = if shadow_enabled {
        Some(shadow_sender)
    } else {
        drop(shadow_sender);
        None
    };
    let context_task = tokio::spawn(
        ContextIngestionWorker {
            config: config.clone(),
            session_id,
            context_analysis_limits,
            previous_context,
            counters: Arc::clone(&context_counters),
            shadow_sender,
            shadow_budget,
            shadow_counters,
        }
        .run(context_receiver),
    );
    let recorder_task = tokio::spawn(
        RunRecorder {
            config,
            session_id,
            parent_operation_id,
            context_sender,
            analysis_enabled,
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
    let transport_task = tokio::spawn(run_transport_dispatcher(transport_receiver, sender.clone()));
    let dropped_context_counters = Arc::clone(&context_counters);
    let durable = DurableEventIngress::new(sender, proxy.background_task_spawner(), move |event| {
        account_durable_event_drop(&dropped_context_counters, analysis_enabled, event);
    });
    let ordering = Arc::new(TransportOrdering::new(transport_sender));
    let proxy = proxy
        .with_metadata_sink(Arc::new(RecorderSink {
            durable: Arc::clone(&durable),
            ordering: Arc::clone(&ordering),
            counters: Arc::clone(&counters),
            context_counters: Arc::clone(&context_counters),
            analysis_slots: Arc::clone(&analysis_slots),
            analysis_enabled,
            active_counters: Arc::clone(&active_counters),
        }))
        .with_observation_sink(Arc::new(RecorderSink {
            durable: Arc::clone(&durable),
            ordering,
            counters: Arc::clone(&counters),
            context_counters: Arc::clone(&context_counters),
            analysis_slots,
            analysis_enabled,
            active_counters: Arc::clone(&active_counters),
        }));
    SpawnedRecorder {
        proxy,
        recorder_task,
        transport_task,
        context_task,
        shadow_task,
        counters,
        context_counters,
        active_counters,
    }
}
