#![allow(
    clippy::multiple_crate_versions,
    clippy::match_wildcard_for_single_variants,
    clippy::unnecessary_wraps,
    clippy::use_debug,
    clippy::format_collect,
    clippy::indexing_slicing,
    clippy::map_unwrap_or,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unused_async,
    clippy::significant_drop_tightening,
    reason = "CLI boundary formats user-facing output and validates bounded fixed-size state"
)]
//! Tracepress command-line boundary.
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    future::Future,
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
use tracepress_compression::{
    BlockKind as CompressionBlockKind, BlockMetadata, BlockOrigin as CompressionBlockOrigin,
    CacheRisk as CompressionCacheRisk, CandidateMetrics, CandidateStatus, CompressionLimits,
    DetectedKind as CompressionDetectedKind, JsonCompactRecords, JsonEmptyNoiseFieldReducer,
    JsonKeyElision, JsonMinify, JsonNoop, JsonReadableTable, JsonRepeatedSubtree,
    JsonRepeatedValueReducer, JsonTabular, ReductionMetrics, ReductionPolicyDecision,
    ReductionStatus, SearchResultReducer, ShadowCompressor, ShellDiagnosticProjectionReducer,
    ShellSemanticFamily, TextLogPrefixFold, TextNoop, TextReadableBlockFold, TextReadableLineFold,
    TextRepeatedLine, TextRepeatedRun, ToolFamily, ToolResultReducer, evaluate_with_estimator,
};
use tracepress_context::{
    ContextAnalysisLimits, ContextAnalysisResult, ContextAnalysisStatus, ContextBlockKind,
    ContextBlockSummary, ContextDeltaRequest, ContextOrigin, ContextRole, DetectedContentKind,
    EstimationRequest, MeasurementApplicability, StructuralHeuristicEstimator,
    TokenEstimateAggregate, TokenEstimator, TokenReconciliation, compute_context_delta,
};
use tracepress_core::{
    AttemptId, CompressionCandidateId, ContextSnapshotId, HttpStatusCode, MaxIpcFrameBytes,
    MaxRequestBodyBytes, MaxResponseBodyBytes, OperationId, RequestId, ResourceLimits,
    ResourceLimitsConfig, SessionId, UuidV7Generator,
};
use tracepress_daemon::{
    ContextAnalysisFinalize, ContextAnalysisMetrics, ContextAppendReceipt,
    ContextCorrelationStatusWire, ControlRequest, ControlResponse, CorrelationDegradation,
    CorrelationStatus, ProviderObservation as ObservationRecord, ProviderObservationOutcome,
    ShadowExperimentCounters, ShadowExperimentManifest,
};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcLimits, IpcRequest, ResponseOutcome, UnixEndpoint,
};
use tracepress_provider::{
    ProviderEndpoint, ProviderRequestKind, ProviderResponseState, ProviderTransport,
    RequestObservation, ResponseObservation,
};
use tracepress_proxy::{
    ActiveCompressionMode, ActiveCompressionObservation, BackgroundTaskSpawner,
    CompactionObservation, ContextAnalysisDropReason, ContextAnalysisMode,
    ContextAnalysisObservation, ContextAnalysisOutcome, DeferredAnalysisMetrics, ForwardId,
    ForwardMetadata, InboundRoute, MetadataSink, MetadataSinkError, ObservationSinkError,
    ProviderObservationSink, ProxyConfig, RequestContextObservation, ShadowAnalysisBody,
    TransparentProxy, TransportFailure,
};
use tracepress_storage::{
    ContextInspection, ContextInspectionBlock, ContextInspectionNamedEstimate,
    ContextSnapshotStatus,
};
use tracepress_storage::{ShadowCacheRisk, ShadowCandidateRecord, ShadowCandidateStatus};

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
/// Transport admission and dispatch share the existing 64-forward correlation bound.
const TRANSPORT_DISPATCH_QUEUE_ITEMS: usize = IN_FLIGHT_FORWARDS;

/// Bound on retired identities kept one by one above the retirement watermark.
///
/// The watermark absorbs the contiguous prefix of retired identities for free, so this bound is
/// only reached when that many identities the proxy never reported an event for sit below the
/// newest retirement. Beyond it the oldest retirement is forgotten, which is the one least
/// likely to still have a half in flight.
const RETIRED_IDENTITIES: usize = 256;
/// Bounded tombstones prevent a late duplicate event from re-opening a terminal analysis.
const TERMINAL_ANALYSIS_IDENTITIES: usize = 256;

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
    fn report(&self) -> String {
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
struct DurableEventIngress<T> {
    sender: tokio::sync::mpsc::Sender<T>,
    state: Mutex<DurableEventIngressState<T>>,
    space: Condvar,
    background: BackgroundTaskSpawner,
    on_drop: Arc<dyn Fn(T) + Send + Sync>,
}

#[derive(Debug)]
struct DurableEventIngressState<T> {
    events: VecDeque<T>,
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
    fn new(
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

    fn try_send(self: &Arc<Self>, event: T) -> Result<(), MetadataSinkError> {
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
    fn send_request(self: &Arc<Self>, event: T) -> Result<(), MetadataSinkError> {
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
    fn queued_len(&self) -> usize {
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
struct ContextIngestionJob {
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
struct AnalysisOutputPermit {
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
struct ContextReceipt {
    forward: ForwardId,
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
#[derive(Clone, Copy)]
struct ContextCoverage {
    token_estimation_eligible: u64,
    token_estimation_observed: u64,
    semantic_detection_eligible: u64,
    semantic_detection_observed: u64,
    correlation_eligible: u64,
    correlation_correlated: u64,
}

struct ContextAnalysisInput {
    forward: ForwardId,
    outcome: ContextAnalysisOutcome,
    shadow_body: Option<ShadowAnalysisBody>,
    analysis_permit: Option<AnalysisOutputPermit>,
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
struct DashboardOptions {
    port: u16,
    fixture: bool,
    large_fixture: bool,
}

/// Accounting for context analyses rejected before context persistence.
#[derive(Debug, Default)]
struct ContextCounters {
    observer_backpressure: AtomicU64,
    deferred_backlog_capacity: AtomicU64,
    resource_limit: AtomicU64,
    malformed: AtomicU64,
    correlation_degraded: AtomicU64,
    unsupported: AtomicU64,
    cancelled: AtomicU64,
    pre_persistence_drop_backpressure: AtomicU64,
    pre_persistence_drop_deferred_backlog_capacity: AtomicU64,
    pre_persistence_drop_resource_limit: AtomicU64,
    pre_persistence_drop_malformed: AtomicU64,
    pre_persistence_drop_correlation: AtomicU64,
    pre_persistence_drop_unsupported: AtomicU64,
    pre_persistence_drop_cancelled: AtomicU64,
    pending_drop_backpressure: AtomicU64,
    pending_drop_deferred_backlog_capacity: AtomicU64,
    pending_drop_resource_limit: AtomicU64,
    pending_drop_malformed: AtomicU64,
    pending_drop_correlation: AtomicU64,
    pending_drop_unsupported: AtomicU64,
    pending_drop_cancelled: AtomicU64,
    analysis_requests_seen: AtomicU64,
    analysis_requests_complete: AtomicU64,
    analysis_requests_partial: AtomicU64,
    analysis_requests_dropped: AtomicU64,
    token_estimation_eligible: AtomicU64,
    token_estimation_observed: AtomicU64,
    semantic_detection_eligible: AtomicU64,
    semantic_detection_observed: AtomicU64,
    correlation_eligible: AtomicU64,
    correlation_correlated: AtomicU64,
    lifecycle: Mutex<ContextLifecycle>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct AnalysisSequence(u64);

impl From<ForwardId> for AnalysisSequence {
    fn from(forward: ForwardId) -> Self {
        Self(forward.get())
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingAnalysisEvidence {
    sequence: AnalysisSequence,
    provider_request_id: RequestId,
    snapshot_id: Option<ContextSnapshotId>,
}

#[derive(Debug, Default)]
struct ContextLifecycle {
    /// Analyses admitted by a sink but not yet terminally classified. Correlation admission
    /// bounds this set to the in-flight identity cap.
    pending: BTreeSet<AnalysisSequence>,
    /// Durable provider/snapshot identities retained while an analysis is pending. This is
    /// bounded alongside `pending` and is used to reconcile a response lost after commit.
    evidence: BTreeMap<AnalysisSequence, PendingAnalysisEvidence>,
    /// Terminal identities above the contiguous watermark, retained to reject reordered
    /// duplicates until the watermark catches up.
    terminal_holes: BTreeSet<AnalysisSequence>,
    terminal_through: Option<AnalysisSequence>,
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

    fn ensure_seen_sequence(&self, sequence: AnalysisSequence) {
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

    fn pending_evidence(&self) -> Vec<PendingAnalysisEvidence> {
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

    fn complete_sequence(&self, sequence: AnalysisSequence) {
        if self.mark_terminal_sequence(sequence) {
            let _counted = self
                .analysis_requests_complete
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn partial(&self, forward: ForwardId) {
        self.partial_sequence(AnalysisSequence::from(forward));
    }

    fn partial_sequence(&self, sequence: AnalysisSequence) {
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
    fn drop_pending(&self, reason: ContextAnalysisDropReason) {
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

    fn report(&self) -> String {
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
fn bounded_record_provider_observation_request(
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

    if !fits(&observation) {
        if let Some(response) = observation.response.as_mut() {
            response.raw_usage = None;
        }
    }
    if !fits(&observation) {
        if let Some(response) = observation.response.as_mut() {
            response.provider_response_id = None;
            response.model = None;
            response.incomplete_reason = None;
            response.error_code = None;
        }
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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct TransportSequence(u64);

/// A bounded handoff from the synchronous metadata sink to the transport dispatcher.
#[derive(Debug)]
enum TransportDispatchJob {
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
struct TransportResponseAdmissionFailure {
    observation: Box<ResponseObservation>,
    count: bool,
}

/// Coordinates bounded transport admission with its terminal provider observation.
///
/// Metadata is placed on the dispatcher queue before its latch becomes visible. A response that
/// observes the latch is therefore queued behind the metadata on the same FIFO dispatcher, while
/// an overflow response is explicitly degraded instead of waiting for an unbounded task.
#[derive(Debug)]
struct TransportOrdering {
    sender: tokio::sync::mpsc::Sender<TransportDispatchJob>,
    pending: Mutex<BTreeMap<TransportSequence, Arc<TransportLatch>>>,
}

#[derive(Debug, Default)]
struct TransportLatch {
    complete: Mutex<bool>,
    wake: Condvar,
}
impl TransportOrdering {
    const fn new(sender: tokio::sync::mpsc::Sender<TransportDispatchJob>) -> Self {
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
    fn try_enqueue_placeholder(&self, sequence: TransportSequence) -> bool {
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
    fn pending_len(&self) -> usize {
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
async fn run_transport_dispatcher(
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
struct RecorderSink {
    durable: Arc<DurableEventIngress<RunEvent>>,
    ordering: Arc<TransportOrdering>,
    counters: Arc<CorrelationCounters>,
    context_counters: Arc<ContextCounters>,
    /// Bounds analyzed outcomes retained between the recorder channel and the context worker.
    analysis_slots: Arc<AnalysisOutputSlots>,
    analysis_enabled: bool,
    active_counters: Arc<ActiveCompressionCounters>,
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
struct RunRecorder {
    config: Config,
    session_id: SessionId,
    parent_operation_id: OperationId,
    context_sender: tokio::sync::mpsc::Sender<ContextIngestionJob>,
    context_counters: Arc<ContextCounters>,
    forwards: BTreeMap<ForwardId, ForwardState>,
    analysis_enabled: bool,
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
    async fn context_analysis(&mut self, input: ContextAnalysisInput) {
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
        if let CorrelationStatus::Degraded(reason) = correlation {
            if !admission_was_counted {
                self.counters.degraded(reason);
            }
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
                if self.pending_context.len() >= IN_FLIGHT_FORWARDS {
                    if let Some((evicted, _receipt)) = self.pending_context.pop_first() {
                        self.context_counters
                            .dropped(evicted, ContextAnalysisDropReason::CorrelationDegraded);
                    }
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

/// The bounded predecessor retained by the context worker for the next context delta.
#[derive(Debug)]
struct PreviousContextSnapshot {
    id: ContextSnapshotId,
    blocks: Vec<ContextBlockSummary>,
    status: ContextAnalysisStatus,
}

const SHADOW_QUEUE_ITEMS: usize = 8;
const SHADOW_QUEUE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
struct ShadowByteBudget {
    used: AtomicU64,
    maximum: u64,
}

impl ShadowByteBudget {
    const fn new(maximum: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            maximum,
        }
    }

    fn try_acquire(self: &Arc<Self>, bytes: u64) -> Option<ShadowBytePermit> {
        let mut observed = self.used.load(Ordering::Relaxed);
        loop {
            let next = observed.checked_add(bytes)?;
            if next > self.maximum {
                return None;
            }
            match self.used.compare_exchange_weak(
                observed,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(ShadowBytePermit {
                        budget: Arc::clone(self),
                        bytes,
                    });
                }
                Err(actual) => observed = actual,
            }
        }
    }
}

#[derive(Debug)]
struct ShadowBytePermit {
    budget: Arc<ShadowByteBudget>,
    bytes: u64,
}

impl Drop for ShadowBytePermit {
    fn drop(&mut self) {
        let previous = self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(previous >= self.bytes, "shadow byte accounting underflow");
    }
}

#[derive(Debug, Default)]
struct ShadowCounters {
    drops: AtomicU64,
    jobs_admitted: AtomicU64,
    jobs_processed: AtomicU64,
    job_drops: AtomicU64,
    candidate_evaluations_attempted: AtomicU64,
    candidate_evaluations_completed: AtomicU64,
    candidate_evaluation_drops: AtomicU64,
    queue_full_drops: AtomicU64,
    byte_budget_drops: AtomicU64,
    work_budget_drops: AtomicU64,
    worker_closed_drops: AtomicU64,
    persistence_drops: AtomicU64,
    recovery_failures: AtomicU64,
    determinism_failures: AtomicU64,
}

#[derive(Debug, Default)]
struct ActiveCompressionCounters {
    attempts: AtomicU64,
    rewrites: AtomicU64,
    evaluated_spans: AtomicU64,
    evaluated_input_bytes: AtomicU64,
    evaluated_candidate_bytes: AtomicU64,
    input_bytes: AtomicU64,
    output_bytes: AtomicU64,
    no_improvement: AtomicU64,
    not_applicable: AtomicU64,
    resource_limits: AtomicU64,
    invalid_inputs: AtomicU64,
    recovery_failures: AtomicU64,
    determinism_failures: AtomicU64,
    internal_errors: AtomicU64,
}

impl ActiveCompressionCounters {
    fn record(&self, observation: &ActiveCompressionObservation) {
        let _attempt = self.attempts.fetch_add(1, Ordering::Relaxed);
        let metrics = &observation.metrics;
        let _spans = self
            .evaluated_spans
            .fetch_add(u64::from(metrics.evaluated_spans), Ordering::Relaxed);
        let _input = self
            .evaluated_input_bytes
            .fetch_add(metrics.evaluated_input_bytes, Ordering::Relaxed);
        let _candidate = self
            .evaluated_candidate_bytes
            .fetch_add(metrics.evaluated_candidate_bytes, Ordering::Relaxed);
        if matches!(
            metrics.status,
            tracepress_compression::ActiveRewriteStatus::InternalError
                | tracepress_compression::ActiveRewriteStatus::RecoveryFailed
        ) && !metrics.deterministic
        {
            let _count = self.determinism_failures.fetch_add(1, Ordering::Relaxed);
        }
        match metrics.status {
            tracepress_compression::ActiveRewriteStatus::Rewritten => {
                let _rewrite = self.rewrites.fetch_add(1, Ordering::Relaxed);
                let _input = self
                    .input_bytes
                    .fetch_add(metrics.input_bytes, Ordering::Relaxed);
                let _output = self
                    .output_bytes
                    .fetch_add(metrics.output_bytes.unwrap_or_default(), Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::NoImprovement => {
                let _count = self.no_improvement.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::NotApplicable => {
                let _count = self.not_applicable.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::ResourceLimit => {
                let _count = self.resource_limits.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::InvalidInput => {
                let _count = self.invalid_inputs.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::RecoveryFailed => {
                let _count = self.recovery_failures.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                let _count = self.internal_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn report(&self) -> String {
        let input = self.input_bytes.load(Ordering::Relaxed);
        let output = self.output_bytes.load(Ordering::Relaxed);
        let reduction = input.saturating_sub(output);
        format!(
            "active_compression_attempts={} active_compression_rewrites={} active_compression_evaluated_spans={} active_compression_evaluated_input_bytes={} active_compression_evaluated_candidate_bytes={} active_compression_input_bytes={} active_compression_output_bytes={} active_compression_reduction_bytes={} active_compression_no_improvement={} active_compression_not_applicable={} active_compression_resource_limits={} active_compression_invalid_inputs={} active_compression_recovery_failures={} active_compression_determinism_failures={} active_compression_internal_errors={}",
            self.attempts.load(Ordering::Relaxed),
            self.rewrites.load(Ordering::Relaxed),
            self.evaluated_spans.load(Ordering::Relaxed),
            self.evaluated_input_bytes.load(Ordering::Relaxed),
            self.evaluated_candidate_bytes.load(Ordering::Relaxed),
            input,
            output,
            reduction,
            self.no_improvement.load(Ordering::Relaxed),
            self.not_applicable.load(Ordering::Relaxed),
            self.resource_limits.load(Ordering::Relaxed),
            self.invalid_inputs.load(Ordering::Relaxed),
            self.recovery_failures.load(Ordering::Relaxed),
            self.determinism_failures.load(Ordering::Relaxed),
            self.internal_errors.load(Ordering::Relaxed),
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum ShadowDropReason {
    QueueFull,
    ByteBudget,
    WorkBudget,
    WorkerClosed,
    Persistence,
}

impl ShadowCounters {
    fn record_job_admitted(&self) {
        let _count = self.jobs_admitted.fetch_add(1, Ordering::Relaxed);
    }

    fn record_job_processed(&self) {
        let _count = self.jobs_processed.fetch_add(1, Ordering::Relaxed);
    }

    fn record_job_drop(&self) {
        let _count = self.job_drops.fetch_add(1, Ordering::Relaxed);
    }

    fn record_candidate_attempt(&self) {
        let _count = self
            .candidate_evaluations_attempted
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_candidate_completed(&self) {
        let _count = self
            .candidate_evaluations_completed
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_candidate_drop(&self) {
        let _count = self
            .candidate_evaluation_drops
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_drop(&self, reason: ShadowDropReason) {
        let _total = self.drops.fetch_add(1, Ordering::Relaxed);
        let counter = match reason {
            ShadowDropReason::QueueFull => &self.queue_full_drops,
            ShadowDropReason::ByteBudget => &self.byte_budget_drops,
            ShadowDropReason::WorkBudget => &self.work_budget_drops,
            ShadowDropReason::WorkerClosed => &self.worker_closed_drops,
            ShadowDropReason::Persistence => &self.persistence_drops,
        };
        let _reason_count = counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct ShadowJob {
    snapshot_id: ContextSnapshotId,
    analysis: ContextAnalysisResult,
    accepted_block_count: u64,
    body: ShadowAnalysisBody,
    _byte_permit: ShadowBytePermit,
}

struct ShadowCompressionWorker {
    config: Config,
    experiment_id: String,
    limits: CompressionLimits,
    context_limits: ContextAnalysisLimits,
    counters: Arc<ShadowCounters>,
}

#[derive(Clone, Debug, Default)]
struct ShadowShapeMetadata {
    provider_readability: String,
    json_root_kind: Option<String>,
    json_array_length_bucket: Option<String>,
    json_object_key_count_bucket: Option<String>,
    json_homogeneity_basis_points: Option<u16>,
    json_primitive_cell_ratio_basis_points: Option<u16>,
    json_nested_cell_ratio_basis_points: Option<u16>,
    text_shape: Option<String>,
}

impl ShadowCompressionWorker {
    async fn run(self, mut jobs: tokio::sync::mpsc::Receiver<ShadowJob>) -> Result<(), String> {
        self.record_manifest("running", None).await?;
        while let Some(job) = jobs.recv().await {
            self.record_job(job).await;
        }
        self.flush_counters().await;
        self.record_manifest("completed", current_timestamp().ok())
            .await
    }

    async fn record_manifest(
        &self,
        status: &str,
        completed_at: Option<String>,
    ) -> Result<(), String> {
        let compressor_set_json = serde_json::to_string(&[
            ("json.noop", 1_u32),
            ("json.minify", 1),
            ("json.tabular", 1),
            ("json.repeated_subtree", 1),
            ("json.readable_table", 1),
            ("json.compact_records", 1),
            ("json.key_elision", 1),
            ("json.empty_noise_fields", 1),
            ("json.repeated_value_elision", 1),
            ("shell.diagnostic_projection", 1),
            ("search.result_projection", 1),
            ("text.noop", 1),
            ("text.repeated_line", 1),
            ("text.repeated_run", 1),
            ("text.readable_line_fold", 1),
            ("text.readable_block_fold", 1),
            ("text.log_prefix_fold", 1),
        ])
        .map_err(|error| error.to_string())?;
        let limits_json = serde_json::to_string(&self.limits).map_err(|error| error.to_string())?;
        let response = control(
            &self.config,
            ControlRequest::RecordShadowExperiment {
                manifest: ShadowExperimentManifest::new(
                    self.experiment_id.clone(),
                    compressor_set_json,
                    option_env!("TRACEPRESS_BUILD_SHA").map(str::to_owned),
                    limits_json,
                    status.to_owned(),
                    current_timestamp()?,
                    completed_at,
                ),
            },
        )
        .await?;
        match response {
            ControlResponse::Ok { .. } => Ok(()),
            _ => Err("daemon rejected shadow experiment manifest".to_owned()),
        }
    }

    async fn record_job(&self, job: ShadowJob) {
        self.counters.record_job_processed();
        let candidates = evaluate_shadow_job(
            &job,
            &self.experiment_id,
            self.limits,
            self.context_limits,
            &self.counters,
        );
        if candidates.is_empty() {
            return;
        }
        self.persist_shadow_candidates(candidates).await;
    }

    async fn persist_shadow_candidates(&self, candidates: Vec<ShadowCandidateRecord>) {
        let mut batch = Vec::new();
        for candidate in candidates {
            let mut proposed = batch.clone();
            proposed.push(candidate.clone());
            if shadow_candidate_batch_fits(&proposed) {
                batch.push(candidate);
                continue;
            }

            if !batch.is_empty() {
                self.persist_shadow_candidate_batch(std::mem::take(&mut batch))
                    .await;
            }
            if shadow_candidate_batch_fits(std::slice::from_ref(&candidate)) {
                batch.push(candidate);
            } else {
                self.report_shadow_persistence_failure(1, "candidate exceeds IPC body limit");
            }
        }
        if !batch.is_empty() {
            self.persist_shadow_candidate_batch(batch).await;
        }
    }

    async fn persist_shadow_candidate_batch(&self, candidates: Vec<ShadowCandidateRecord>) {
        let batch_size = candidates.len();
        let result = control(
            &self.config,
            ControlRequest::RecordShadowCandidates { candidates },
        )
        .await;
        match result {
            Ok(ControlResponse::Ok { .. }) => {}
            Ok(ControlResponse::Error { message }) => {
                self.report_shadow_persistence_failure(batch_size, &message);
            }
            Ok(ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. }) => {
                self.report_shadow_persistence_failure(batch_size, "unexpected daemon response");
            }
            Err(error) => self.report_shadow_persistence_failure(batch_size, &error),
        }
    }

    fn report_shadow_persistence_failure(&self, batch_size: usize, error: &str) {
        // Keep persistence diagnostics metadata-only. Candidate payloads are never logged because
        // shadow inputs may originate from a private workspace.
        eprintln!("shadow candidate persistence failed: batch_size={batch_size} error={error}");
        self.counters.record_drop(ShadowDropReason::Persistence);
    }

    async fn flush_counters(&self) {
        let _ = control(
            &self.config,
            ControlRequest::RecordShadowCounters {
                counters: ShadowExperimentCounters::new(
                    self.experiment_id.clone(),
                    0,
                    self.counters.drops.swap(0, Ordering::AcqRel),
                    self.counters.jobs_admitted.swap(0, Ordering::AcqRel),
                    self.counters.jobs_processed.swap(0, Ordering::AcqRel),
                    self.counters.job_drops.swap(0, Ordering::AcqRel),
                    self.counters
                        .candidate_evaluations_attempted
                        .swap(0, Ordering::AcqRel),
                    self.counters
                        .candidate_evaluations_completed
                        .swap(0, Ordering::AcqRel),
                    self.counters
                        .candidate_evaluation_drops
                        .swap(0, Ordering::AcqRel),
                    self.counters.queue_full_drops.swap(0, Ordering::AcqRel),
                    self.counters.byte_budget_drops.swap(0, Ordering::AcqRel),
                    self.counters.work_budget_drops.swap(0, Ordering::AcqRel),
                    self.counters.worker_closed_drops.swap(0, Ordering::AcqRel),
                    self.counters.persistence_drops.swap(0, Ordering::AcqRel),
                    self.counters.recovery_failures.swap(0, Ordering::AcqRel),
                    self.counters.determinism_failures.swap(0, Ordering::AcqRel),
                ),
            },
        )
        .await;
    }
}

/// Returns whether a shadow-candidate control request stays below the conservative IPC body
/// budget. `IpcRequest` embeds the body as a JSON byte array, so the quarter-frame bound leaves
/// room for that expansion and the authenticated envelope.
fn shadow_candidate_batch_fits(candidates: &[ShadowCandidateRecord]) -> bool {
    let body_limit = usize::try_from(BODY_BYTES / 4).unwrap_or(0);
    let request = ControlRequest::RecordShadowCandidates {
        candidates: candidates.to_vec(),
    };
    serde_json::to_vec(&request).is_ok_and(|body| body.len() <= body_limit)
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "shadow inputs keep independent bounds and accounting explicit"
)]
fn evaluate_shadow_job(
    job: &ShadowJob,
    experiment_id: &str,
    limits: CompressionLimits,
    context_limits: ContextAnalysisLimits,
    counters: &ShadowCounters,
) -> Vec<ShadowCandidateRecord> {
    let accepted = usize::try_from(job.accepted_block_count)
        .unwrap_or(job.analysis.blocks.len())
        .min(job.analysis.blocks.len());
    let generator = UuidV7Generator::new();
    let estimator = StructuralHeuristicEstimator::new();
    let json_compressors: [&dyn ShadowCompressor; 7] = [
        &JsonNoop,
        &JsonMinify,
        &JsonTabular,
        &JsonRepeatedSubtree,
        &JsonReadableTable,
        &JsonCompactRecords,
        &JsonKeyElision,
    ];
    let text_compressors: [&dyn ShadowCompressor; 6] = [
        &TextNoop,
        &TextRepeatedLine,
        &TextRepeatedRun,
        &TextReadableLineFold,
        &TextReadableBlockFold,
        &TextLogPrefixFold,
    ];
    let mut records = Vec::new();
    let mut remaining_work = limits.max_shadow_work_units;
    for block in &job.analysis.blocks[..accepted] {
        let metadata = shadow_block_metadata(block, job.body.len());
        let compressors: &[&dyn ShadowCompressor] = match metadata.detected_kind {
            CompressionDetectedKind::Json if metadata.is_tool_result_json() => &json_compressors,
            CompressionDetectedKind::PlainText if metadata.is_tool_result_plain_text() => {
                &text_compressors
            }
            _ => continue,
        };
        let Some(content) = shadow_block_content(job.body.as_ref(), block) else {
            continue;
        };
        let shape = classify_shadow_shape(&content, metadata.detected_kind);
        let shell_generic = block
            .tool_name
            .as_ref()
            .map(|name| ToolFamily::from_tool_name(Some(name.as_str())))
            == Some(ToolFamily::ShellGeneric);
        // Search eligibility is proved by the transient ToolCall/ToolResult pair, not by the
        // provider's tool-name label alone. Native providers may call the shell surface by a
        // different name while still carrying an unambiguous bounded `rg` command. The command
        // and output are discarded after this in-memory classification.
        let shell_family =
            classify_shell_semantic_family(&job.analysis.blocks, block, job.body.as_ref());
        let reduction_reducers: [(&dyn ToolResultReducer, bool); 4] = [
            (&JsonEmptyNoiseFieldReducer, true),
            (&JsonRepeatedValueReducer, true),
            (&ShellDiagnosticProjectionReducer, shell_generic),
            (
                &SearchResultReducer,
                // Provider-native shell labels and call-id correlation can be absent even when
                // the ToolResult is a JSON envelope. The reducer itself requires one canonical
                // Search payload, so evaluating JSON ToolResults in shadow is still fail-closed
                // and gives an explicit `not_applicable` record instead of silent missingness.
                shell_family == ShellSemanticFamily::Search || metadata.is_tool_result_json(),
            ),
        ];
        let max_candidates = usize::try_from(limits.max_candidates_per_block).unwrap_or(usize::MAX);
        for (candidate_index, compressor) in compressors.iter().enumerate() {
            if candidate_index >= max_candidates {
                counters.record_candidate_drop();
                continue;
            }
            let work = u64::try_from(content.len()).unwrap_or(u64::MAX).max(1);
            let Some(remaining) = remaining_work.checked_sub(work) else {
                counters.record_candidate_drop();
                counters.record_drop(ShadowDropReason::WorkBudget);
                continue;
            };
            remaining_work = remaining;
            counters.record_candidate_attempt();
            let candidate =
                evaluate_with_estimator(*compressor, metadata, &content, &limits, |bytes| {
                    estimator
                        .estimate(&EstimationRequest {
                            model: None,
                            content: bytes,
                            limits: context_limits,
                        })
                        .tokens()
                });
            let metrics = candidate.metrics();
            counters.record_candidate_completed();
            if matches!(metrics.status, CandidateStatus::RecoveryFailed) {
                let _counted = counters.recovery_failures.fetch_add(1, Ordering::Relaxed);
            }
            if !metrics.deterministic && metrics.candidate_fingerprint.is_some() {
                let _counted = counters
                    .determinism_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
            records.push(shadow_candidate_record(
                &generator,
                experiment_id,
                job.snapshot_id,
                u64::from(block.ordinal),
                metrics,
                &shape,
            ));
        }
        for (reduction_index, (reduction_reducer, enabled)) in reduction_reducers.iter().enumerate()
        {
            if !enabled {
                continue;
            }
            let candidate_index = json_compressors.len().saturating_add(reduction_index);
            if candidate_index >= max_candidates {
                counters.record_candidate_drop();
                continue;
            }
            if !matches!(
                reduction_reducer
                    .policy(metadata, u64::try_from(content.len()).unwrap_or(u64::MAX)),
                ReductionPolicyDecision::Reduce | ReductionPolicyDecision::KeepFullWithCandidate
            ) {
                continue;
            }
            // A reducer is evaluated twice for determinism. Reserve a conservative shared
            // budget for both passes so the shadow worker can never outrun its job bound.
            let work = u64::try_from(content.len())
                .unwrap_or(u64::MAX)
                .max(1)
                .saturating_mul(2);
            let Some(remaining) = remaining_work.checked_sub(work) else {
                counters.record_candidate_drop();
                counters.record_drop(ShadowDropReason::WorkBudget);
                continue;
            };
            remaining_work = remaining;
            counters.record_candidate_attempt();
            let reduced = reduction_reducer.reduce(&content, &limits);
            let repeat = reduction_reducer.reduce(&content, &limits);
            let deterministic = reduced.metrics().visible_fingerprint
                == repeat.metrics().visible_fingerprint
                && reduced.metrics().status == repeat.metrics().status;
            let visible_estimate = reduced.visible().and_then(|bytes| {
                estimator
                    .estimate(&EstimationRequest {
                        model: None,
                        content: bytes,
                        limits: context_limits,
                    })
                    .tokens()
            });
            let reduced = reduced
                .with_estimates(metadata.input_estimated_tokens, visible_estimate)
                .with_deterministic(deterministic);
            counters.record_candidate_completed();
            let metrics = reduced.metrics();
            records.push(shadow_reduction_candidate_record(
                &generator,
                experiment_id,
                job.snapshot_id,
                u64::from(block.ordinal),
                metrics,
                &shape,
            ));
        }
    }
    records
}

fn classify_shell_semantic_family(
    blocks: &[tracepress_context::ContextBlockDraft],
    result: &tracepress_context::ContextBlockDraft,
    body: &[u8],
) -> ShellSemanticFamily {
    let command = result
        .tool_call_id
        .as_ref()
        .and_then(|result_call_id| {
            blocks.iter().find(|candidate| {
                candidate.kind == ContextBlockKind::ToolCall
                    && candidate
                        .tool_call_id
                        .as_ref()
                        .is_some_and(|call_id| call_id.as_str() == result_call_id.as_str())
            })
        })
        .and_then(|call| shadow_block_content(body, call));
    let output = shadow_block_content(body, result);
    ShellSemanticFamily::from_transient_signals(command.as_deref(), output.as_deref())
}

fn shadow_block_content(
    body: &[u8],
    block: &tracepress_context::ContextBlockDraft,
) -> Option<Vec<u8>> {
    let [raw_start, raw_end] = block
        .content_span
        .unwrap_or([block.locator.raw_value_start, block.locator.raw_value_end]);
    let start = usize::try_from(raw_start).ok()?;
    let end = usize::try_from(raw_end).ok()?;
    let raw = body.get(start..end)?;
    if raw.first() == Some(&b'"') {
        serde_json::from_slice::<String>(raw)
            .ok()
            .map(String::into_bytes)
    } else {
        Some(raw.to_vec())
    }
}

fn shadow_block_metadata(
    block: &tracepress_context::ContextBlockDraft,
    request_analysis_bytes: usize,
) -> BlockMetadata {
    let origin = match block.origin {
        ContextOrigin::ToolGenerated => CompressionBlockOrigin::ToolGenerated,
        ContextOrigin::HumanAuthored => CompressionBlockOrigin::HumanAuthored,
        ContextOrigin::AgentGenerated => CompressionBlockOrigin::AgentGenerated,
        ContextOrigin::ToolSchema => CompressionBlockOrigin::ToolSchema,
        ContextOrigin::ProviderManaged => CompressionBlockOrigin::ProviderManaged,
        _ => CompressionBlockOrigin::Other,
    };
    let kind = if block.kind == ContextBlockKind::ToolResult {
        CompressionBlockKind::ToolResult
    } else {
        CompressionBlockKind::Other
    };
    let detected_kind = match block.detection_result.as_ref().map(|value| value.kind) {
        Some(DetectedContentKind::Json) => CompressionDetectedKind::Json,
        Some(DetectedContentKind::PlainText) => CompressionDetectedKind::PlainText,
        Some(DetectedContentKind::Unknown) | None => CompressionDetectedKind::Unknown,
        _ => CompressionDetectedKind::Other,
    };
    BlockMetadata::new(
        origin,
        kind,
        detected_kind,
        block.token_estimate.as_ref().map(|value| value.tokens),
        block.locator.raw_value_start,
        u64::try_from(request_analysis_bytes).unwrap_or(u64::MAX),
        None,
        None,
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "the metadata-only shape classifier keeps all bounded shape rules together"
)]
fn classify_shadow_shape(
    input: &[u8],
    detected_kind: CompressionDetectedKind,
) -> ShadowShapeMetadata {
    let provider_readability = "unknown";
    match detected_kind {
        CompressionDetectedKind::Json => {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(input) else {
                return ShadowShapeMetadata {
                    provider_readability: provider_readability.to_owned(),
                    ..ShadowShapeMetadata::default()
                };
            };
            let mut shape = ShadowShapeMetadata {
                provider_readability: provider_readability.to_owned(),
                ..ShadowShapeMetadata::default()
            };
            match &value {
                serde_json::Value::Array(rows) => {
                    shape.json_array_length_bucket = Some(length_bucket(rows.len()));
                    if rows.iter().all(serde_json::Value::is_object) {
                        shape.json_root_kind = Some("array_object".to_owned());
                        let objects: Vec<&serde_json::Map<String, serde_json::Value>> = rows
                            .iter()
                            .filter_map(serde_json::Value::as_object)
                            .collect();
                        let first_keys = objects
                            .first()
                            .map(|object| object.keys().cloned().collect::<BTreeSet<_>>())
                            .unwrap_or_default();
                        let homogeneous = objects
                            .iter()
                            .filter(|object| {
                                object.keys().cloned().collect::<BTreeSet<_>>() == first_keys
                            })
                            .count();
                        shape.json_homogeneity_basis_points =
                            Some(ratio_basis_points(homogeneous, objects.len()));
                        shape.json_object_key_count_bucket = Some(length_bucket(first_keys.len()));
                        let mut primitive = 0_usize;
                        let mut nested = 0_usize;
                        for object in &objects {
                            for cell in object.values() {
                                if cell.is_array() || cell.is_object() {
                                    nested = nested.saturating_add(1);
                                } else {
                                    primitive = primitive.saturating_add(1);
                                }
                            }
                        }
                        let total = primitive.saturating_add(nested);
                        shape.json_primitive_cell_ratio_basis_points =
                            Some(ratio_basis_points(primitive, total));
                        shape.json_nested_cell_ratio_basis_points =
                            Some(ratio_basis_points(nested, total));
                    } else if rows
                        .iter()
                        .all(|value| value.is_object() || value.is_array())
                    {
                        shape.json_root_kind = Some("array_nested".to_owned());
                    } else if rows
                        .iter()
                        .all(|value| !value.is_object() && !value.is_array())
                    {
                        shape.json_root_kind = Some("array_scalar".to_owned());
                    } else {
                        shape.json_root_kind = Some("array_heterogeneous".to_owned());
                    }
                }
                serde_json::Value::Object(object) => {
                    shape.json_root_kind = Some("object".to_owned());
                    shape.json_object_key_count_bucket = Some(length_bucket(object.len()));
                }
                serde_json::Value::String(_) => shape.json_root_kind = Some("string".to_owned()),
                serde_json::Value::Number(_) => shape.json_root_kind = Some("number".to_owned()),
                serde_json::Value::Bool(_) => shape.json_root_kind = Some("boolean".to_owned()),
                serde_json::Value::Null => shape.json_root_kind = Some("null".to_owned()),
            }
            shape
        }
        CompressionDetectedKind::PlainText => {
            let text = String::from_utf8_lossy(input);
            let lines: Vec<&str> = text.lines().collect();
            let mut counts = BTreeMap::<&str, usize>::new();
            for line in &lines {
                let count = counts.entry(line).or_default();
                *count = count.saturating_add(1);
            }
            let duplicate_lines = counts.values().any(|count| *count > 1);
            let log_like = lines
                .iter()
                .filter(|line| line.split_whitespace().count() >= 3 && line.contains(' '))
                .count();
            ShadowShapeMetadata {
                provider_readability: provider_readability.to_owned(),
                text_shape: Some(
                    if duplicate_lines {
                        "duplicate_lines"
                    } else if log_like.saturating_mul(2) >= lines.len().max(1) {
                        "logs_like"
                    } else {
                        "plain"
                    }
                    .to_owned(),
                ),
                ..ShadowShapeMetadata::default()
            }
        }
        _ => ShadowShapeMetadata {
            provider_readability: provider_readability.to_owned(),
            ..ShadowShapeMetadata::default()
        },
    }
}

fn length_bucket(length: usize) -> String {
    match length {
        0 => "0".to_owned(),
        1..=1 => "1".to_owned(),
        2..=9 => "2-9".to_owned(),
        10..=99 => "10-99".to_owned(),
        100..=999 => "100-999".to_owned(),
        _ => "1000+".to_owned(),
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "bounded ratio conversion saturates before narrowing"
)]
fn ratio_basis_points(numerator: usize, denominator: usize) -> u16 {
    if denominator == 0 {
        return 0;
    }
    u16::try_from((numerator.saturating_mul(10_000) / denominator).min(10_000)).unwrap_or(10_000)
}

fn compressor_provider_readability(compressor_id: &str) -> String {
    match compressor_id {
        "json.readable_table"
        | "json.compact_records"
        | "json.key_elision"
        | "text.readable_line_fold"
        | "text.readable_block_fold"
        | "text.log_prefix_fold" => "human_readable_structured".to_owned(),
        "json.noop" | "json.minify" | "text.noop" => "provider_compatible_control".to_owned(),
        _ => "opaque_custom_encoding".to_owned(),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "candidate association metadata remains explicit"
)]
fn shadow_candidate_record(
    generator: &UuidV7Generator,
    experiment_id: &str,
    snapshot_id: ContextSnapshotId,
    block_ordinal: u64,
    metrics: &CandidateMetrics,
    shape: &ShadowShapeMetadata,
) -> ShadowCandidateRecord {
    let mut shape = shape.clone();
    shape.provider_readability = compressor_provider_readability(&metrics.compressor_id);
    ShadowCandidateRecord {
        candidate_id: CompressionCandidateId::generate(generator),
        experiment_id: experiment_id.to_owned(),
        snapshot_id,
        block_ordinal,
        compressor_id: metrics.compressor_id.clone(),
        compressor_version: metrics.compressor_version.to_string(),
        status: storage_shadow_status(metrics.status),
        input_bytes: metrics.input_bytes,
        output_bytes: metrics.output_bytes,
        bytes_delta: metrics
            .bytes_delta
            .and_then(|value| u64::try_from(value).ok()),
        input_estimated_tokens: metrics.input_estimated_tokens,
        output_estimated_tokens: metrics.output_estimated_tokens,
        estimated_token_delta: metrics
            .estimated_token_delta
            .and_then(|value| u64::try_from(value).ok()),
        processing_us: metrics.processing_us,
        reversible: metrics.reversible,
        recovery_verified: metrics.recovery_verified,
        deterministic: metrics.deterministic,
        original_fingerprint: metrics.original_fingerprint.to_vec().into_boxed_slice(),
        candidate_fingerprint: metrics
            .candidate_fingerprint
            .map(|value| value.to_vec().into_boxed_slice()),
        recovered_fingerprint: metrics
            .recovery_verified
            .then(|| metrics.original_fingerprint.to_vec().into_boxed_slice()),
        first_modified_offset: metrics.first_modified_offset,
        preserved_prefix_bytes: metrics.preserved_prefix_bytes,
        preserved_prefix_ratio_basis_points: metrics.preserved_prefix_ratio_basis_points,
        cache_risk: storage_cache_risk(metrics.cache_risk),
        provider_readability: shape.provider_readability,
        json_root_kind: shape.json_root_kind,
        json_array_length_bucket: shape.json_array_length_bucket,
        json_object_key_count_bucket: shape.json_object_key_count_bucket,
        json_homogeneity_basis_points: shape.json_homogeneity_basis_points,
        json_primitive_cell_ratio_basis_points: shape.json_primitive_cell_ratio_basis_points,
        json_nested_cell_ratio_basis_points: shape.json_nested_cell_ratio_basis_points,
        text_shape: shape.text_shape,
        verified_at_us: current_timestamp_us().ok(),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "reduction association metadata remains explicit"
)]
fn shadow_reduction_candidate_record(
    generator: &UuidV7Generator,
    experiment_id: &str,
    snapshot_id: ContextSnapshotId,
    block_ordinal: u64,
    metrics: &ReductionMetrics,
    shape: &ShadowShapeMetadata,
) -> ShadowCandidateRecord {
    ShadowCandidateRecord {
        candidate_id: CompressionCandidateId::generate(generator),
        experiment_id: experiment_id.to_owned(),
        snapshot_id,
        block_ordinal,
        compressor_id: metrics.reducer_id.to_owned(),
        compressor_version: metrics.reducer_version.to_string(),
        status: storage_reduction_status(metrics.status),
        input_bytes: metrics.input_bytes,
        output_bytes: metrics.visible_bytes,
        bytes_delta: metrics.gross_bytes_delta,
        input_estimated_tokens: metrics.input_estimated_tokens,
        output_estimated_tokens: metrics.visible_estimated_tokens,
        estimated_token_delta: metrics.gross_estimated_token_delta,
        processing_us: metrics.processing_us,
        reversible: metrics.recovery_available,
        recovery_verified: metrics.recovery_verified,
        deterministic: metrics.deterministic,
        original_fingerprint: metrics.original_fingerprint.to_vec().into_boxed_slice(),
        candidate_fingerprint: metrics
            .visible_fingerprint
            .map(|value| value.to_vec().into_boxed_slice()),
        recovered_fingerprint: metrics
            .recovery_verified
            .then(|| metrics.original_fingerprint.to_vec().into_boxed_slice()),
        first_modified_offset: metrics.first_modified_offset,
        preserved_prefix_bytes: metrics.preserved_prefix_bytes,
        preserved_prefix_ratio_basis_points: None,
        cache_risk: ShadowCacheRisk::Unknown,
        provider_readability: metrics.provider_readability.to_owned(),
        json_root_kind: shape.json_root_kind.clone(),
        json_array_length_bucket: shape.json_array_length_bucket.clone(),
        json_object_key_count_bucket: shape.json_object_key_count_bucket.clone(),
        json_homogeneity_basis_points: shape.json_homogeneity_basis_points,
        json_primitive_cell_ratio_basis_points: shape.json_primitive_cell_ratio_basis_points,
        json_nested_cell_ratio_basis_points: shape.json_nested_cell_ratio_basis_points,
        text_shape: shape.text_shape.clone(),
        verified_at_us: current_timestamp_us().ok(),
    }
}

const fn storage_shadow_status(status: CandidateStatus) -> ShadowCandidateStatus {
    match status {
        CandidateStatus::Applicable => ShadowCandidateStatus::Applicable,
        CandidateStatus::NotApplicable => ShadowCandidateStatus::NotApplicable,
        CandidateStatus::NoImprovement => ShadowCandidateStatus::NoImprovement,
        CandidateStatus::ResourceLimit => ShadowCandidateStatus::ResourceLimit,
        CandidateStatus::InvalidInput => ShadowCandidateStatus::InvalidInput,
        CandidateStatus::RecoveryFailed => ShadowCandidateStatus::RecoveryFailed,
        _ => ShadowCandidateStatus::InternalError,
    }
}

const fn storage_reduction_status(status: ReductionStatus) -> ShadowCandidateStatus {
    match status {
        ReductionStatus::Applicable => ShadowCandidateStatus::Applicable,
        ReductionStatus::NotApplicable => ShadowCandidateStatus::NotApplicable,
        ReductionStatus::NoImprovement => ShadowCandidateStatus::NoImprovement,
        ReductionStatus::ResourceLimit => ShadowCandidateStatus::ResourceLimit,
        ReductionStatus::InvalidInput => ShadowCandidateStatus::InvalidInput,
        _ => ShadowCandidateStatus::InternalError,
    }
}

const fn storage_cache_risk(risk: CompressionCacheRisk) -> ShadowCacheRisk {
    match risk {
        CompressionCacheRisk::Low => ShadowCacheRisk::Low,
        CompressionCacheRisk::Medium => ShadowCacheRisk::Medium,
        CompressionCacheRisk::High => ShadowCacheRisk::High,
        _ => ShadowCacheRisk::Unknown,
    }
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
    shadow_sender: Option<tokio::sync::mpsc::Sender<ShadowJob>>,
    shadow_budget: Arc<ShadowByteBudget>,
    shadow_counters: Arc<ShadowCounters>,
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
        flush_context_drops(&self.config, self.session_id, &self.counters).await;
    }

    async fn begin_context(
        &self,
        provider_request_id: RequestId,
        inference_operation_id: OperationId,
    ) -> Result<(ContextSnapshotId, u64), ContextAnalysisDropReason> {
        let started_at_us = current_timestamp_us()
            .map_err(|_error| ContextAnalysisDropReason::CorrelationDegraded)?;
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
        .await;
        let response = response.map_err(|_error| ContextAnalysisDropReason::CorrelationDegraded)?;
        match response {
            ControlResponse::Ok {
                context_snapshot_id: Some(snapshot_id),
                ..
            } => Ok((snapshot_id, started_at_us)),
            ControlResponse::Error { .. } | ControlResponse::Context { .. } => {
                Err(ContextAnalysisDropReason::CorrelationDegraded)
            }
            _ => Err(ContextAnalysisDropReason::Unsupported),
        }
    }

    async fn abort_context(
        &self,
        snapshot_id: ContextSnapshotId,
        reason: ContextAnalysisDropReason,
    ) {
        let Ok(completed_at_us) = current_timestamp_us() else {
            return;
        };
        let _ = control(
            &self.config,
            ControlRequest::AbortContextAnalysis {
                snapshot_id,
                reason,
                completed_at_us,
            },
        )
        .await;
    }

    async fn append_batch(
        &self,
        input: ContextAppendBatchInput,
    ) -> Result<ContextAppendReceipt, ContextAnalysisDropReason> {
        let ContextAppendBatchInput {
            snapshot_id,
            sequence,
            blocks,
        } = input;
        let response = control(
            &self.config,
            ControlRequest::AppendContextBlocks {
                snapshot_id,
                sequence,
                blocks,
            },
        )
        .await
        .map_err(|_error| ContextAnalysisDropReason::CorrelationDegraded)?;
        match response {
            ControlResponse::Ok {
                context_append_receipt: Some(receipt),
                ..
            } => Ok(receipt),
            ControlResponse::Error { .. } | ControlResponse::Context { .. } => {
                Err(ContextAnalysisDropReason::CorrelationDegraded)
            }
            _ => Err(ContextAnalysisDropReason::Unsupported),
        }
    }

    async fn append_context_blocks(
        &self,
        input: ContextAppendInput<'_>,
    ) -> Result<(ContextAnalysisStatus, u64), ContextAnalysisDropReason> {
        let ContextAppendInput {
            snapshot_id,
            analysis,
            mut status,
        } = input;
        let mut sequence = 0_u32;
        let max_batches =
            u32::try_from(self.context_analysis_limits.max_batches.get()).unwrap_or(u32::MAX);
        // `IpcRequest` serializes its opaque body as a JSON byte array; four frame bytes per
        // logical byte is the conservative bound that keeps the enclosing request under 64 KiB.
        let body_limit = usize::try_from(BODY_BYTES / 4).unwrap_or(0);
        let total_blocks = u64::try_from(analysis.blocks.len()).unwrap_or(u64::MAX);
        let mut batch = Vec::new();
        let mut accepted_block_count = 0_u64;
        let mut capacity_exhausted = false;

        for block in analysis.blocks.iter().cloned() {
            if capacity_exhausted || sequence >= max_batches {
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
                break;
            };
            if encoded.len() <= body_limit {
                batch.push(block);
                continue;
            }
            if batch.is_empty() {
                // A singleton that cannot fit is an explicit resource-limited prefix.
                break;
            }
            let outgoing = std::mem::take(&mut batch);
            let receipt = self
                .append_batch(ContextAppendBatchInput {
                    snapshot_id,
                    sequence,
                    blocks: outgoing,
                })
                .await?;
            accepted_block_count = receipt.accepted_block_count;
            sequence = receipt.next_sequence;
            capacity_exhausted = receipt.capacity.is_exhausted();
            if capacity_exhausted {
                break;
            }
            let singleton = vec![block.clone()];
            let singleton_request = ControlRequest::AppendContextBlocks {
                snapshot_id,
                sequence,
                blocks: singleton,
            };
            let Ok(singleton_encoded) = serde_json::to_vec(&singleton_request) else {
                break;
            };
            if singleton_encoded.len() > body_limit {
                break;
            }
            batch.push(block);
        }
        if !batch.is_empty() && !capacity_exhausted && sequence < max_batches {
            let receipt = self
                .append_batch(ContextAppendBatchInput {
                    snapshot_id,
                    sequence,
                    blocks: batch,
                })
                .await?;
            accepted_block_count = receipt.accepted_block_count;
        }
        if accepted_block_count < total_blocks {
            status = ContextAnalysisStatus::ResourceLimit;
        }
        Ok((status, accepted_block_count))
    }
    async fn record_context(&mut self, job: ContextIngestionJob) {
        let ContextIngestionJob {
            forward,
            provider_request_id,
            attempt_id,
            inference_operation_id,
            analysis,
            shadow_body,
            analysis_permit,
            provider_input_tokens,
            provider_usage_comparable,
            correlation,
        } = job;
        let _analysis_permit = analysis_permit;
        let (snapshot_id, started_at_us) = match self
            .begin_context(provider_request_id, inference_operation_id)
            .await
        {
            Ok(value) => value,
            Err(reason) => {
                self.counters.dropped(forward, reason);
                return;
            }
        };
        self.counters.remember_snapshot(forward, snapshot_id);
        let initial_status = if matches!(correlation, CorrelationStatus::Correlated) {
            analysis.status
        } else {
            ContextAnalysisStatus::CorrelationDegraded
        };
        let (status, accepted_block_count) = match self
            .append_context_blocks(ContextAppendInput {
                snapshot_id,
                analysis: &analysis,
                status: initial_status,
            })
            .await
        {
            Ok(value) => value,
            Err(reason) => {
                self.abort_context(snapshot_id, reason).await;
                self.counters.dropped(forward, reason);
                return;
            }
        };
        let finalized = self
            .finalize_context(ContextFinalizationInput {
                forward,
                snapshot_id,
                started_at_us,
                attempt_id,
                analysis: &analysis,
                provider_input_tokens,
                provider_usage_comparable,
                correlation,
                status,
                accepted_block_count,
            })
            .await;
        if finalized {
            self.enqueue_shadow(snapshot_id, analysis, accepted_block_count, shadow_body);
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the handoff retains explicit snapshot association"
    )]
    fn enqueue_shadow(
        &self,
        snapshot_id: ContextSnapshotId,
        analysis: ContextAnalysisResult,
        accepted_block_count: u64,
        body: Option<ShadowAnalysisBody>,
    ) {
        let (Some(sender), Some(body)) = (&self.shadow_sender, body) else {
            return;
        };
        let bytes = u64::try_from(body.len()).unwrap_or(u64::MAX);
        let Some(byte_permit) = self.shadow_budget.try_acquire(bytes) else {
            self.shadow_counters.record_job_drop();
            self.shadow_counters
                .record_drop(ShadowDropReason::ByteBudget);
            return;
        };
        let job = ShadowJob {
            snapshot_id,
            analysis,
            accepted_block_count,
            body,
            _byte_permit: byte_permit,
        };
        match sender.try_send(job) {
            Ok(()) => self.shadow_counters.record_job_admitted(),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_job)) => {
                self.shadow_counters.record_job_drop();
                self.shadow_counters
                    .record_drop(ShadowDropReason::QueueFull);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_job)) => {
                self.shadow_counters.record_job_drop();
                self.shadow_counters
                    .record_drop(ShadowDropReason::WorkerClosed);
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the finalization boundary assembles all durable context evidence atomically"
    )]
    fn build_context_finalize(
        &self,
        input: &ContextFinalizationInput<'_>,
    ) -> Option<(
        Vec<ContextBlockSummary>,
        ContextAnalysisFinalize,
        ContextCoverage,
    )> {
        let analysis = input.analysis;
        let accepted_len = usize::try_from(input.accepted_block_count)
            .unwrap_or(analysis.blocks.len())
            .min(analysis.blocks.len());
        let accepted_blocks = &analysis.blocks[..accepted_len];
        let current_blocks = context_block_summaries(accepted_blocks);
        let delta = self.previous_context.as_ref().map(|previous| {
            compute_context_delta(ContextDeltaRequest {
                previous_snapshot_id: previous.id,
                current_snapshot_id: input.snapshot_id,
                previous_analysis_status: previous.status,
                current_analysis_status: input.status,
                previous: &previous.blocks,
                current: &current_blocks,
                limits: self.context_analysis_limits,
            })
        });
        let (mut metrics, visible_estimated_tokens) = context_metrics(
            accepted_blocks,
            input.status,
            delta
                .as_ref()
                .and_then(|value| value.common_prefix_estimated_tokens),
        );
        let unknown_block_count = accepted_blocks
            .iter()
            .filter(|block| block.kind == ContextBlockKind::Unknown)
            .count();
        metrics.unknown_block_count = Some(u64::try_from(unknown_block_count).unwrap_or(u64::MAX));
        metrics.semantic_coverage_basis_points = if input.status != ContextAnalysisStatus::Complete
            || accepted_len != analysis.blocks.len()
            || accepted_blocks.is_empty()
        {
            None
        } else {
            #[allow(
                clippy::arithmetic_side_effects,
                reason = "the bounded block count makes this basis-point projection intentional"
            )]
            let basis_points = accepted_blocks
                .len()
                .saturating_sub(unknown_block_count)
                .saturating_mul(10_000)
                / accepted_blocks.len();
            u16::try_from(basis_points).ok()
        };
        let token_eligible = accepted_blocks
            .iter()
            .filter(|block| {
                block.token_estimation_applicability == MeasurementApplicability::Eligible
            })
            .count();
        let token_observed = accepted_blocks
            .iter()
            .filter(|block| block.token_estimate.is_some())
            .count();
        let semantic_eligible = accepted_blocks
            .iter()
            .filter(|block| block.detection_applicability == MeasurementApplicability::Eligible)
            .count();
        let semantic_observed = accepted_blocks
            .iter()
            .filter(|block| block.detection_result.is_some())
            .count();
        let coverage = ContextCoverage {
            token_estimation_eligible: u64::try_from(token_eligible).unwrap_or(u64::MAX),
            token_estimation_observed: u64::try_from(token_observed).unwrap_or(u64::MAX),
            semantic_detection_eligible: u64::try_from(semantic_eligible).unwrap_or(u64::MAX),
            semantic_detection_observed: u64::try_from(semantic_observed).unwrap_or(u64::MAX),
            correlation_eligible: 1,
            correlation_correlated: u64::from(matches!(
                input.correlation,
                CorrelationStatus::Correlated
            )),
        };
        let correlation_status = if matches!(input.correlation, CorrelationStatus::Correlated) {
            ContextCorrelationStatusWire::Correlated
        } else {
            ContextCorrelationStatusWire::Degraded
        };
        let reconciliation = TokenReconciliation::reconcile(
            input.snapshot_id,
            analysis.visibility,
            visible_estimated_tokens,
            input.provider_input_tokens,
            input.provider_usage_comparable
                && matches!(
                    input.status,
                    ContextAnalysisStatus::Complete
                        | ContextAnalysisStatus::Partial
                        | ContextAnalysisStatus::ResourceLimit
                ),
        );
        let finalize = ContextAnalysisFinalize::builder(
            input.snapshot_id,
            input.status,
            current_timestamp_us().unwrap_or(input.started_at_us),
        )
        .request_content_hash(Some(analysis.request_content_hash))
        .analysis_content_hash(Some(analysis.analysis_content_hash))
        .explicit_block_count(Some(input.accepted_block_count))
        .analyzed_bytes(Some(analysis.analyzed_bytes))
        .skipped_bytes(Some(analysis.skipped_bytes))
        .visibility(analysis.visibility)
        .duplicate_key_detected(Some(analysis.duplicate_key_detected))
        .reference_resolved_locally(Some(analysis.visibility_facts.reference_resolved_locally))
        .correlation_status(correlation_status)
        .metrics(metrics)
        .delta(delta)
        .reconciliation(reconciliation)
        .attempt_id(Some(input.attempt_id))
        .build()
        .ok()?;
        Some((current_blocks, finalize, coverage))
    }
    async fn finalize_context(&mut self, input: ContextFinalizationInput<'_>) -> bool {
        let forward = input.forward;
        let snapshot_id = input.snapshot_id;
        let status = input.status;
        let Some((current_blocks, finalize, coverage)) = self.build_context_finalize(&input) else {
            self.abort_context(snapshot_id, ContextAnalysisDropReason::CorrelationDegraded)
                .await;
            self.counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return false;
        };
        let finalized = control(
            &self.config,
            ControlRequest::FinalizeContextAnalysis {
                summary: Box::new(finalize),
            },
        )
        .await;
        match finalized {
            Ok(ControlResponse::Ok { .. }) => {
                self.counters.observe_coverage(coverage);
                match status {
                    ContextAnalysisStatus::Complete => self.counters.complete(forward),
                    ContextAnalysisStatus::Partial => self.counters.partial(forward),
                    ContextAnalysisStatus::ResourceLimit => self.counters.partial_with_drop_reason(
                        forward,
                        ContextAnalysisDropReason::ResourceLimit,
                    ),
                    ContextAnalysisStatus::Malformed => self
                        .counters
                        .partial_with_drop_reason(forward, ContextAnalysisDropReason::Malformed),
                    ContextAnalysisStatus::ObserverBackpressure => {
                        self.counters.partial_with_drop_reason(
                            forward,
                            ContextAnalysisDropReason::ObserverBackpressure,
                        );
                    }
                    ContextAnalysisStatus::CorrelationDegraded => {
                        self.counters.partial_with_drop_reason(
                            forward,
                            ContextAnalysisDropReason::CorrelationDegraded,
                        );
                    }
                    ContextAnalysisStatus::Cancelled => self
                        .counters
                        .partial_with_drop_reason(forward, ContextAnalysisDropReason::Cancelled),
                    _ => self
                        .counters
                        .partial_with_drop_reason(forward, ContextAnalysisDropReason::Unsupported),
                }
                self.previous_context = Some(PreviousContextSnapshot {
                    id: snapshot_id,
                    blocks: current_blocks,
                    status,
                });
                true
            }
            Ok(
                ControlResponse::Error { .. }
                | ControlResponse::Context { .. }
                | ControlResponse::ContextStatus { .. },
            )
            | Err(_) => {
                self.abort_context(snapshot_id, ContextAnalysisDropReason::CorrelationDegraded)
                    .await;
                self.counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
                false
            }
        }
    }
}
async fn flush_context_drops(config: &Config, session_id: SessionId, counters: &ContextCounters) {
    let reasons = [
        ContextAnalysisDropReason::ObserverBackpressure,
        ContextAnalysisDropReason::DeferredBacklogCapacity,
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
        let dropped = counters.take_pending_drop_count(reason);
        if dropped == 0 {
            continue;
        }
        let _ = control(
            config,
            ControlRequest::RecordContextAnalysisDropped {
                session_id,
                reason,
                dropped,
                observed_at_us,
            },
        )
        .await;
    }
}

type ObservationRecordParts = (
    ObservationRecord,
    Option<ContextAnalysisOutcome>,
    Option<ShadowAnalysisBody>,
    Option<AnalysisOutputPermit>,
    Option<u64>,
    bool,
);

fn observation_record(
    semantic: SemanticRecord,
    ended_at: String,
    correlation: CorrelationStatus,
) -> Option<ObservationRecordParts> {
    let SemanticRecord {
        pending,
        response,
        context,
        shadow_body,
        status_code,
        transport_failure,
        analysis_permit,
    } = semantic;
    // The parser always measures its bounded input; an unmeasured body is never invented.
    // A decoder rejected by the bounded analysis capacity did not present bytes to the semantic
    // parser. The provider row still keeps the exact accepted wire count, while the semantic
    // request length remains NULL in `RequestObservation`.
    let request_bytes = pending
        .request
        .request_bytes
        .or(pending.request.wire_bytes)?;
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
        shadow_body,
        analysis_permit,
        provider_input_tokens,
        provider_usage_comparable,
    ))
}

fn context_block_summaries(
    blocks: &[tracepress_context::ContextBlockDraft],
) -> Vec<ContextBlockSummary> {
    blocks
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
    blocks: &[tracepress_context::ContextBlockDraft],
    status: ContextAnalysisStatus,
    stable_explicit_prefix_estimate: Option<u64>,
) -> (ContextAnalysisMetrics, Option<u64>) {
    let analysis_complete = matches!(status, ContextAnalysisStatus::Complete);
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

    for block in blocks {
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
    metrics.explicit_bytes = Some(blocks.iter().map(|block| block.raw_bytes).sum());
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
    analysis_enabled: bool,
    shadow_enabled: bool,
    shadow_experiment_id: String,
    previous_context: Option<PreviousContextSnapshot>,
}
/// The recorder of one run: its proxy, its workers, and the counters it publishes.
struct SpawnedRecorder {
    proxy: TransparentProxy,
    recorder_task: tokio::task::JoinHandle<Result<(), String>>,
    transport_task: tokio::task::JoinHandle<()>,
    context_task: tokio::task::JoinHandle<Result<(), String>>,
    shadow_task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    /// Correlation accounting the run reports, readable whether or not the workers finished.
    counters: Arc<CorrelationCounters>,
    /// Context analyses rejected by the bounded context queue.
    context_counters: Arc<ContextCounters>,
    active_counters: Arc<ActiveCompressionCounters>,
}

/// Installs both auxiliary sinks and starts the provider and context workers.
#[allow(
    clippy::too_many_lines,
    reason = "worker ownership and channel closure ordering stay together"
)]
fn spawn_recorder(recording: RunRecording, proxy: TransparentProxy) -> SpawnedRecorder {
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
    /// Launches the local, read-only Tracepress Observatory.
    Ui {
        /// Local loopback port.
        #[arg(long, default_value_t = tracepress_dashboard_api::DEFAULT_PORT)]
        port: u16,
        /// Use synthetic metadata instead of the operational database.
        #[arg(long)]
        fixture: bool,
        /// Generate the 1,000-session / 10,000-request / 100,000-block smoke fixture.
        #[arg(long, requires = "fixture")]
        large_fixture: bool,
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

/// Reconciles pending analyses against durable daemon state before the forced-drain drop
/// boundary. A status without a completion timestamp is still active and must remain pending.
async fn reconcile_pending_context(config: &Config, counters: &ContextCounters) {
    reconcile_pending_context_with(counters, |evidence| async move {
        let request = ControlRequest::ContextStatus {
            request_id: Some(evidence.provider_request_id),
            snapshot_id: evidence.snapshot_id,
        };
        match tokio::time::timeout(Duration::from_millis(250), control(config, request)).await {
            Ok(Ok(ControlResponse::ContextStatus {
                snapshot_status: Some(status),
            })) => Some(status),
            _ => None,
        }
    })
    .await;
}

async fn reconcile_pending_context_with<F, Fut>(counters: &ContextCounters, mut lookup: F)
where
    F: FnMut(PendingAnalysisEvidence) -> Fut,
    Fut: Future<Output = Option<ContextSnapshotStatus>>,
{
    for evidence in counters.pending_evidence() {
        let Some(status) = lookup(evidence).await else {
            continue;
        };
        if status.completed_at_us.is_none() {
            continue;
        }
        if status.status == "complete" {
            counters.complete_sequence(evidence.sequence);
        } else {
            counters.partial_sequence(evidence.sequence);
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
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
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
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
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

fn shadow_compression_enabled() -> Result<bool, String> {
    let value = std::env::var("TRACEPRESS_SHADOW_COMPRESSION")
        .unwrap_or_else(|_| "off".to_owned())
        .to_ascii_lowercase();
    match value.as_str() {
        "off" => Ok(false),
        "on" => Ok(true),
        _ => Err(format!(
            "TRACEPRESS_SHADOW_COMPRESSION must be `off` or `on`, got `{value}`"
        )),
    }
}

fn active_compression_mode() -> Result<ActiveCompressionMode, String> {
    let value = std::env::var("TRACEPRESS_ACTIVE_COMPRESSION")
        .unwrap_or_else(|_| "off".to_owned())
        .to_ascii_lowercase();
    match value.as_str() {
        "off" => Ok(ActiveCompressionMode::Off),
        "json.minify" | "json_minify" => Ok(ActiveCompressionMode::JsonMinify),
        "search.result_projection" | "search_projection" => {
            Ok(ActiveCompressionMode::SearchProjection)
        }
        _ => Err(format!(
            "TRACEPRESS_ACTIVE_COMPRESSION must be `off`, `json.minify`, or `search.result_projection`, got `{value}`"
        )),
    }
}

fn shadow_experiment_id() -> Result<String, String> {
    let value = std::env::var("TRACEPRESS_SHADOW_EXPERIMENT_ID")
        .unwrap_or_else(|_| "shadow-pilot-001".to_owned());
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(value)
    } else {
        Err("TRACEPRESS_SHADOW_EXPERIMENT_ID must be 1-128 ASCII identifier characters".to_owned())
    }
}

fn is_codex_agent(agent: &str) -> bool {
    std::path::Path::new(agent)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "codex" || name == "codex.exe")
}

fn provider_transport_from_value(value: &str) -> Result<ProviderTransport, String> {
    match value.to_ascii_lowercase().as_str() {
        "openai_public_api" | "public" => Ok(ProviderTransport::OpenAiPublicApi),
        "chatgpt_codex_subscription" | "subscription" | "codex_subscription" => {
            Ok(ProviderTransport::ChatGptCodexSubscription)
        }
        _ => Err(format!(
            "TRACEPRESS_PROVIDER_TRANSPORT must be `openai_public_api` or `chatgpt_codex_subscription`, got `{value}`"
        )),
    }
}

fn provider_transport_for_agent(agent: &str) -> Result<ProviderTransport, String> {
    if let Ok(value) = std::env::var("TRACEPRESS_PROVIDER_TRANSPORT") {
        return provider_transport_from_value(&value);
    }
    if is_codex_agent(agent) {
        Ok(ProviderTransport::ChatGptCodexSubscription)
    } else {
        Ok(ProviderTransport::OpenAiPublicApi)
    }
}

fn provider_endpoint(transport: ProviderTransport) -> Result<ProviderEndpoint, String> {
    match transport {
        ProviderTransport::OpenAiPublicApi => {
            let upstream = std::env::var("TRACEPRESS_UPSTREAM")
                .map_err(|_| "TRACEPRESS_UPSTREAM is required by `tracepress run`".to_owned())?;
            ProviderEndpoint::new(&upstream).map_err(|error| error.to_string())
        }
        ProviderTransport::ChatGptCodexSubscription => {
            if std::env::var_os("TRACEPRESS_UPSTREAM").is_some() {
                return Err(
                    "TRACEPRESS_UPSTREAM cannot override the fixed ChatGPT Codex subscription endpoint"
                        .to_owned(),
                );
            }
            Ok(ProviderEndpoint::chatgpt_codex_subscription())
        }
        _ => Err("unsupported provider transport".to_owned()),
    }
}

const MEASUREMENT_METADATA_PREFIX: &str = "TRACEPRESS_MEASUREMENT_METADATA=";
const SCHEDULER_METRICS_PREFIX: &str = "TRACEPRESS_SCHEDULER_METRICS=";

fn measurement_metadata_line(
    measurement_run_id: Option<&str>,
    session_id: SessionId,
    started_at: &str,
) -> Option<String> {
    let measurement_run_id =
        measurement_run_id.filter(|value| !value.is_empty() && value.len() <= 128)?;
    let metadata = serde_json::json!({
        "measurement_run_id": measurement_run_id,
        "session_id": session_id.to_string(),
        "tracepress_pid": std::process::id(),
        "started_at": started_at,
    });
    Some(format!(
        "{MEASUREMENT_METADATA_PREFIX}{}",
        serde_json::to_string(&metadata).ok()?
    ))
}

fn scheduler_metrics_line(metrics: &DeferredAnalysisMetrics) -> String {
    let metrics = serde_json::json!({
        "deferred_queue_items": metrics.queue_items,
        "deferred_queue_bytes": metrics.queue_bytes,
        "deferred_high_water_items": metrics.high_water_items,
        "deferred_high_water_bytes": metrics.high_water_bytes,
        "analysis_admitted_total": metrics.deferred_total,
        "analysis_deferred_total": metrics.deferred_due_to_active_forwards_total,
        "processed_deferred_total": metrics.processed_deferred_total,
        "backlog_capacity_drops": metrics.backlog_capacity_drops,
        "analysis_wait_us": metrics.analysis_wait_us,
    });
    format!("{SCHEDULER_METRICS_PREFIX}{metrics}")
}

fn configure_codex_subscription(args: &mut Vec<String>, proxy_address: std::net::SocketAddr) {
    let base_url = format!("http://{proxy_address}/v1");
    let overrides = [
        "-c".to_owned(),
        "model_provider=tracepress_subscription".to_owned(),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.name=OpenAI".to_owned(),
        "-c".to_owned(),
        format!("model_providers.tracepress_subscription.base_url=\"{base_url}\""),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.wire_api=\"responses\"".to_owned(),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.requires_openai_auth=true".to_owned(),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.supports_websockets=false".to_owned(),
    ];
    let insertion = args
        .iter()
        .position(|argument| argument == "exec")
        .map_or(0, |index| index.saturating_add(1));
    let _removed = args.splice(insertion..insertion, overrides);
}

#[allow(
    clippy::too_many_lines,
    reason = "the existing run path keeps lifecycle cleanup and reporting ordered"
)]
async fn run_agent(config: &Config, agent: String, args: Vec<String>) -> Result<(), String> {
    if !daemon_running(config).await? {
        return Err("daemon is not running; run `tracepress daemon start` first".to_owned());
    }
    let transport = provider_transport_for_agent(&agent)?;
    let endpoint = provider_endpoint(transport)?;
    let resource_limits = proxy_resource_limits(8 * 1024 * 1024, 32 * 1024 * 1024)?;
    let analysis_mode = context_analysis_mode()?;
    let shadow_enabled = shadow_compression_enabled()?;
    let active_mode = active_compression_mode()?;
    if shadow_enabled && !matches!(analysis_mode, ContextAnalysisMode::Shadow) {
        return Err("shadow compression requires TRACEPRESS_CONTEXT_ANALYSIS=shadow".to_owned());
    }
    if !matches!(active_mode, ActiveCompressionMode::Off)
        && !matches!(analysis_mode, ContextAnalysisMode::Shadow)
    {
        return Err("active compression requires TRACEPRESS_CONTEXT_ANALYSIS=shadow".to_owned());
    }
    let shadow_experiment_id = shadow_experiment_id()?;
    let context_queue_items = usize::try_from(resource_limits.max_ipc_queue_items.get())
        .map_err(|error| format!("context queue capacity does not fit usize: {error}"))?;
    let context_analysis_limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)
        .map_err(|error| error.to_string())?;
    let proxy = TransparentProxy::new(
        ProxyConfig::new(endpoint, resource_limits, analysis_mode)
            .map(|config| config.with_active_compression_mode(active_mode))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let proxy_address = listener.local_addr().map_err(|error| error.to_string())?;
    let started_at = current_timestamp()?;
    let response = control(
        config,
        ControlRequest::StartSession {
            started_at: started_at.clone(),
        },
    )
    .await?;
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
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    };
    if let Some(metadata) = measurement_metadata_line(
        std::env::var("TRACEPRESS_MEASUREMENT_RUN_ID")
            .ok()
            .as_deref(),
        session.session_id,
        &started_at,
    ) {
        println!("{metadata}");
    }
    let SpawnedRecorder {
        proxy,
        mut recorder_task,
        mut transport_task,
        mut context_task,
        mut shadow_task,
        counters,
        context_counters,
        active_counters,
    } = spawn_recorder(
        RunRecording {
            config: config.clone(),
            session_id: session.session_id,
            parent_operation_id,
            context_queue_items,
            context_analysis_limits,
            analysis_enabled: matches!(analysis_mode, ContextAnalysisMode::Shadow),
            shadow_enabled,
            shadow_experiment_id,
            previous_context: None,
        },
        proxy,
    );
    let background_proxy = proxy.clone();
    let proxy_task = tokio::spawn(async move { serve(listener, proxy.router()).await });
    let base_url = format!("http://{proxy_address}/v1");
    let mut command = Command::new(&agent);
    let mut args = args;
    if matches!(transport, ProviderTransport::ChatGptCodexSubscription) && is_codex_agent(&agent) {
        configure_codex_subscription(&mut args, proxy_address);
    }
    let status_result = command
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
    let (recorded, drain_timed_out) = match tokio::time::timeout(RECORDER_DRAIN_TIMEOUT, async {
        background_proxy.wait_for_background_tasks().await;
        // The proxy task was aborted above, so this is the scheduler's final drain barrier: no
        // new forwards can mutate these counters while recorder/context evidence is drained.
        let deferred_metrics = background_proxy.deferred_analysis_metrics();
        println!(
            "deferred_queue_items={}\ndeferred_queue_bytes={}\ndeferred_high_water_items={}\ndeferred_high_water_bytes={}\nanalysis_admitted_total={}\nanalysis_deferred_total={}\nprocessed_deferred_total={}\nbacklog_capacity_drops={}\nanalysis_wait_us={}",
            deferred_metrics.queue_items,
            deferred_metrics.queue_bytes,
            deferred_metrics.high_water_items,
            deferred_metrics.high_water_bytes,
            deferred_metrics.deferred_total,
            deferred_metrics.deferred_due_to_active_forwards_total,
            deferred_metrics.processed_deferred_total,
            deferred_metrics.backlog_capacity_drops,
            deferred_metrics.analysis_wait_us,
        );
        // The recorder's channel must be closed before it is drained; otherwise the recorder
        // cannot observe end-of-run and wait for more events forever.
        drop(background_proxy);
        let drain_result =
            drain_recorders(
                &mut recorder_task,
                &mut transport_task,
                &mut context_task,
                &mut shadow_task,
            )
            .await;
        // Emit the machine-readable scheduler snapshot only after recorder/context drain. The
        // collector publishes it only after the child exits, binding it to this run identity.
        println!("{}", scheduler_metrics_line(&deferred_metrics));
        drain_result
    })
    .await
    {
        Ok(result) => (result, false),
        Err(_elapsed) => {
            // Reconcile any snapshot that committed before its response was lost. Active
            // snapshots are intentionally left pending until FinishSession gives the daemon a
            // chance to finalize them as durable partial outcomes.
            reconcile_pending_context(config, &context_counters).await;
            transport_task.abort();
            recorder_task.abort();
            context_task.abort();
            if let Some(task) = shadow_task.as_mut() {
                task.abort();
            }
            let _ = (&mut transport_task).await;
            let _ = (&mut recorder_task).await;
            let _ = (&mut context_task).await;
            if let Some(task) = shadow_task.as_mut() {
                let _ = task.await;
            }
            (
                Err(format!(
                    "provider observer/recorder drain exceeded {}s",
                    RECORDER_DRAIN_TIMEOUT.as_secs()
                )),
                true,
            )
        }
    };
    let ended_at = current_timestamp()?;
    let finalization = control(
        config,
        ControlRequest::FinishSession {
            session_id: session.session_id,
            ended_at,
        },
    )
    .await;
    if drain_timed_out {
        // FinishSession may have finalized active snapshots as durable partials. Reconcile only
        // after that boundary, then classify the genuinely unresolved remainder as dropped.
        reconcile_pending_context(config, &context_counters).await;
        context_counters.drop_pending(ContextAnalysisDropReason::Cancelled);
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            flush_context_drops(config, session.session_id, &context_counters),
        )
        .await;
    }
    println!("{}", counters.report());
    println!("{}", context_counters.report());
    println!("{}", active_counters.report());
    let status = status_result.map_err(|error| format!("cannot launch agent {agent}: {error}"))?;
    recorded.map_err(|error| format!("forward recording failed: {error}"))?;
    match finalization? {
        ControlResponse::Ok { .. } => {}
        ControlResponse::Error { message } => {
            return Err(format!(
                "agent finished but session finalization failed: {message}"
            ));
        }
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    }
    match status.code() {
        Some(code) if code != 0 => Err(format!("agent exited with status {code}")),
        Some(_) => Ok(()),
        None => Err("agent terminated by signal".to_owned()),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "each independently owned worker is drained explicitly"
)]
async fn drain_recorders(
    recorder_task: &mut tokio::task::JoinHandle<Result<(), String>>,
    transport_task: &mut tokio::task::JoinHandle<()>,
    context_task: &mut tokio::task::JoinHandle<Result<(), String>>,
    shadow_task: &mut Option<tokio::task::JoinHandle<Result<(), String>>>,
) -> Result<(), String> {
    let recorder_result = (&mut *recorder_task)
        .await
        .map_err(|error| format!("recording worker failed: {error}"))
        .and_then(|result| result);
    let _transport_result = (&mut *transport_task).await;
    // Always wait for the context worker after the recorder has stopped, even when the
    // provider worker reported an IPC error.
    let context_result = (&mut *context_task)
        .await
        .map_err(|error| format!("context worker failed: {error}"))
        .and_then(|result| result);
    let shadow_result = if let Some(task) = shadow_task.as_mut() {
        task.await
            .map_err(|error| format!("shadow worker failed: {error}"))
            .and_then(|result| result)
    } else {
        Ok(())
    };
    recorder_result.and(context_result).and(shadow_result)
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
    let transport = std::env::var("TRACEPRESS_PROVIDER_TRANSPORT")
        .map_or(Ok(ProviderTransport::OpenAiPublicApi), |value| {
            provider_transport_from_value(&value)
        })?;
    let endpoint = provider_endpoint(transport)?;
    let resource_limits = proxy_resource_limits(8 * 1024 * 1024, 32 * 1024 * 1024)?;
    let analysis_mode = context_analysis_mode()?;
    let active_mode = active_compression_mode()?;
    if !matches!(active_mode, ActiveCompressionMode::Off)
        && !matches!(analysis_mode, ContextAnalysisMode::Shadow)
    {
        return Err("active compression requires TRACEPRESS_CONTEXT_ANALYSIS=shadow".to_owned());
    }
    let proxy = TransparentProxy::new(
        ProxyConfig::new(endpoint, resource_limits, analysis_mode)
            .map(|config| config.with_active_compression_mode(active_mode))
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
    let background_proxy = proxy.clone();
    let serve_result = serve(listener, proxy.router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| error.to_string());
    let drain_result = tokio::time::timeout(
        RECORDER_DRAIN_TIMEOUT,
        background_proxy.wait_for_background_tasks(),
    )
    .await;
    if let Err(error) = serve_result {
        let _ = drain_result;
        return Err(error);
    }
    match drain_result {
        Ok(()) => Ok(()),
        Err(_elapsed) => {
            let metrics = background_proxy.deferred_analysis_metrics();
            Err(format!(
                "proxy observer drain exceeded {}s (queue_items={}, queue_bytes={})",
                RECORDER_DRAIN_TIMEOUT.as_secs(),
                metrics.queue_items,
                metrics.queue_bytes,
            ))
        }
    }
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
        ControlResponse::Ok { .. } | ControlResponse::ContextStatus { .. } => {
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
    println!(
        "  unknown blocks: {}",
        optional_number(inspection.coverage.unknown_block_count)
    );
    println!(
        "  semantic coverage: {}",
        inspection
            .coverage
            .semantic_coverage_basis_points
            .map_or_else(
                || "unknown".to_owned(),
                |value| { format!("{:.2}%", f64::from(value) / 100.0) }
            )
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
        CommandKind::Ui {
            port,
            fixture,
            large_fixture,
        } => {
            dashboard(
                &config,
                DashboardOptions {
                    port,
                    fixture,
                    large_fixture,
                },
            )
            .await
        }
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

async fn dashboard(config: &Config, options: DashboardOptions) -> Result<(), String> {
    let fixture_database = options
        .fixture
        .then(|| tracepress_dashboard_api::fixture_database(options.large_fixture))
        .transpose()
        .map_err(|error| error.to_string())?;
    let database_path = fixture_database.as_ref().map_or_else(
        || config.database.clone(),
        |database| database.path().to_path_buf(),
    );
    let reports_path = std::env::var_os("TRACEPRESS_REPORTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reports"));
    let assets_path = std::env::var_os("TRACEPRESS_DASHBOARD_ASSETS")
        .map(PathBuf::from)
        .or_else(discover_dashboard_assets);
    let bind = std::net::SocketAddr::from(([127, 0, 0, 1], options.port));
    println!("Tracepress Observatory");
    println!("http://{bind}");
    let mut dashboard_config =
        tracepress_dashboard_api::DashboardConfig::local(database_path, reports_path);
    dashboard_config.bind = bind;
    dashboard_config.assets_path = assets_path;
    tracepress_dashboard_api::serve(dashboard_config)
        .await
        .map_err(|error| error.to_string())
}

fn discover_dashboard_assets() -> Option<PathBuf> {
    [
        PathBuf::from("target/dx/tracepress-dashboard/release/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-dashboard/release/web/public",
        ),
        PathBuf::from("target/dx/tracepress-dashboard/debug/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-dashboard/debug/web/public",
        ),
        PathBuf::from("target/dx/tracepress-observatory/release/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-observatory/release/web/public",
        ),
        PathBuf::from("target/dx/tracepress-observatory/debug/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-observatory/debug/web/public",
        ),
    ]
    .into_iter()
    .find(|path| path.join("index.html").is_file())
}
#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    use super::{
        AnalysisSequence, BackgroundTaskSpawner, CONTEXT_INGESTION_QUEUE_HARD_CAP,
        CompressionCandidateId, Config, ContextAnalysisDropReason, ContextAnalysisInput,
        ContextCounters, ContextReceipt, ContextSnapshotId, ContextSnapshotStatus, ControlRequest,
        CorrelationCounters, CorrelationStatus, DeferredAnalysisMetrics, DurableEventIngress,
        ObservationRecord, OperationId, PendingAnalysisEvidence, RECORDER_QUEUE_ITEMS, RequestId,
        RunRecorder, SessionId, ShadowCacheRisk, ShadowCandidateRecord, ShadowCandidateStatus,
        TERMINAL_ANALYSIS_IDENTITIES, UuidV7Generator, bounded_record_provider_observation_request,
        configure_codex_subscription, context_ingestion_queue_capacity, measurement_metadata_line,
        reconcile_pending_context_with, scheduler_metrics_line, shadow_candidate_batch_fits,
    };

    use tracepress_provider::{ObservationInput, ObservationLimits, parse_request, parse_response};
    use tracepress_proxy::{ContextAnalysisOutcome, ForwardId};

    fn synthetic_shadow_candidate(
        generator: &UuidV7Generator,
        provider_readability: String,
    ) -> ShadowCandidateRecord {
        ShadowCandidateRecord {
            candidate_id: CompressionCandidateId::generate(generator),
            experiment_id: "test-shadow-batching".to_owned(),
            snapshot_id: ContextSnapshotId::generate(generator),
            block_ordinal: 0,
            compressor_id: "json.noop".to_owned(),
            compressor_version: "1".to_owned(),
            status: ShadowCandidateStatus::NoImprovement,
            input_bytes: 128,
            output_bytes: Some(128),
            bytes_delta: Some(0),
            input_estimated_tokens: Some(32),
            output_estimated_tokens: Some(32),
            estimated_token_delta: Some(0),
            processing_us: 1,
            reversible: true,
            recovery_verified: true,
            deterministic: true,
            original_fingerprint: vec![0_u8; 32].into_boxed_slice(),
            candidate_fingerprint: Some(vec![0_u8; 32].into_boxed_slice()),
            recovered_fingerprint: Some(vec![0_u8; 32].into_boxed_slice()),
            first_modified_offset: None,
            preserved_prefix_bytes: Some(128),
            preserved_prefix_ratio_basis_points: Some(10_000),
            cache_risk: ShadowCacheRisk::Unknown,
            provider_readability,
            json_root_kind: None,
            json_array_length_bucket: None,
            json_object_key_count_bucket: None,
            json_homogeneity_basis_points: None,
            json_primitive_cell_ratio_basis_points: None,
            json_nested_cell_ratio_basis_points: None,
            text_shape: None,
            verified_at_us: Some(1),
        }
    }

    #[test]
    fn shadow_candidate_batches_are_bounded_by_ipc_body_size() {
        let generator = UuidV7Generator::new();
        let candidate = synthetic_shadow_candidate(&generator, "r".repeat(1_000));
        assert!(shadow_candidate_batch_fits(std::slice::from_ref(
            &candidate
        )));
        assert!(!shadow_candidate_batch_fits(&vec![candidate; 16]));
    }

    #[test]
    fn codex_subscription_overrides_follow_the_exec_subcommand() {
        let mut args = vec!["exec".to_owned(), "--ephemeral".to_owned()];
        configure_codex_subscription(
            &mut args,
            std::net::SocketAddr::from(([127, 0, 0, 1], 43_191)),
        );

        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "-c");
        assert_eq!(args[2], "model_provider=tracepress_subscription");
        assert!(args.iter().any(|argument| {
            argument
                == "model_providers.tracepress_subscription.base_url=\"http://127.0.0.1:43191/v1\""
        }));
        assert_eq!(args[13], "--ephemeral");
    }

    #[test]
    fn measurement_lines_bind_scheduler_metrics_to_run_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let session_id = SessionId::generate(&UuidV7Generator::new());
        let metadata = measurement_metadata_line(Some("run-123"), session_id, "unix-ms:1")
            .ok_or_else(|| std::io::Error::other("bounded run id should produce metadata"))?;
        let metadata_payload = metadata
            .strip_prefix("TRACEPRESS_MEASUREMENT_METADATA=")
            .ok_or_else(|| std::io::Error::other("metadata prefix"))?;
        let metadata_json: serde_json::Value = serde_json::from_str(metadata_payload)?;
        assert_eq!(metadata_json["measurement_run_id"], "run-123");
        assert_eq!(metadata_json["session_id"], session_id.to_string());
        assert_eq!(metadata_json["started_at"], "unix-ms:1");
        assert!(metadata_json["tracepress_pid"].as_u64().is_some());

        let metrics = {
            let mut value = DeferredAnalysisMetrics::default();
            value.queue_items = 0;
            value.queue_bytes = 0;
            value.high_water_items = 2;
            value.high_water_bytes = 128;
            value.deferred_total = 7;
            value.deferred_due_to_active_forwards_total = 3;
            value.processed_deferred_total = 7;
            value.backlog_capacity_drops = 0;
            value.analysis_wait_us = 42;
            scheduler_metrics_line(&value)
        };
        let metrics_payload = metrics
            .strip_prefix("TRACEPRESS_SCHEDULER_METRICS=")
            .ok_or_else(|| std::io::Error::other("metrics prefix"))?;
        let metrics_json: serde_json::Value = serde_json::from_str(metrics_payload)?;
        assert_eq!(metrics_json["analysis_admitted_total"], 7);
        assert_eq!(metrics_json["analysis_deferred_total"], 3);
        assert_eq!(metrics_json["processed_deferred_total"], 7);
        assert_eq!(metrics_json["deferred_queue_items"], 0);
        Ok(())
    }

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

    #[tokio::test]
    async fn durable_event_ingress_defers_when_recorder_channel_is_full() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let ingress =
            DurableEventIngress::new(sender.clone(), BackgroundTaskSpawner::new(), |_| {});

        assert!(sender.send(1).await.is_ok());
        assert!(ingress.try_send(2).is_ok());

        let first = receiver.recv().await.unwrap_or(u64::MAX);
        let second = match tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await {
            Ok(Some(event)) => event,
            _ => u64::MAX,
        };
        assert_eq!(first, 1);
        assert_eq!(second, 2);
    }

    #[tokio::test]
    async fn durable_event_ingress_accounts_events_when_recorder_closes() {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_for_handler = Arc::clone(&dropped);
        let ingress = DurableEventIngress::new(
            sender.clone(),
            BackgroundTaskSpawner::new(),
            move |_event: u64| {
                let _ = dropped_for_handler.fetch_add(1, Ordering::Relaxed);
            },
        );

        assert!(sender.send(1).await.is_ok());
        assert!(ingress.try_send(2).is_ok());
        assert!(ingress.try_send(3).is_ok());
        drop(receiver);

        let result = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if dropped.load(Ordering::Relaxed) == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(result.is_ok());
        assert!(ingress.try_send(4).is_err());
    }

    #[tokio::test]
    async fn request_admission_waits_for_bounded_ingress_space() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let ingress =
            DurableEventIngress::new(sender.clone(), BackgroundTaskSpawner::new(), |_| {});

        assert!(sender.send(0).await.is_ok());
        {
            let mut state = ingress
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for event in 1..=RECORDER_QUEUE_ITEMS {
                state.events.push_back(event as u64);
            }
        }
        assert_eq!(ingress.queued_len(), RECORDER_QUEUE_ITEMS);

        let waiting_ingress = Arc::clone(&ingress);
        let waiting = tokio::task::spawn_blocking(move || {
            waiting_ingress.send_request((RECORDER_QUEUE_ITEMS + 1) as u64)
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!waiting.is_finished());

        {
            let mut state = ingress
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _removed = state.events.pop_front();
        }
        ingress.space.notify_one();

        for _ in 0..=RECORDER_QUEUE_ITEMS {
            assert!(receiver.recv().await.is_some());
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn pending_context_receipt_wins_over_retired_identity() {
        let ids = UuidV7Generator::new();
        let forward = ForwardId::default();
        let counters = Arc::new(ContextCounters::default());
        let (context_sender, _context_receiver) = tokio::sync::mpsc::channel(1);
        let mut recorder = RunRecorder {
            config: Config {
                root: std::path::PathBuf::new(),
                database: std::path::PathBuf::new(),
                socket: std::path::PathBuf::new(),
                credential: std::path::PathBuf::new(),
                ready: std::path::PathBuf::new(),
            },
            session_id: SessionId::generate(&ids),
            parent_operation_id: OperationId::generate(&ids),
            context_sender,
            context_counters: Arc::clone(&counters),
            forwards: std::collections::BTreeMap::new(),
            analysis_enabled: true,
            retired_orphans: std::collections::BTreeMap::new(),
            pending_context: std::collections::BTreeMap::new(),
            retired: std::collections::BTreeSet::from([forward]),
            retired_through: None,
            counters: Arc::new(CorrelationCounters::default()),
        };
        let _previous = recorder.pending_context.insert(
            forward,
            ContextReceipt {
                forward,
                provider_request_id: RequestId::generate(&ids),
                attempt_id: tracepress_core::AttemptId::generate(&ids),
                inference_operation_id: OperationId::generate(&ids),
                provider_input_tokens: None,
                provider_usage_comparable: false,
                correlation: CorrelationStatus::Correlated,
            },
        );

        recorder
            .context_analysis(ContextAnalysisInput {
                forward,
                outcome: ContextAnalysisOutcome::Dropped(ContextAnalysisDropReason::Malformed),
                shadow_body: None,
                analysis_permit: None,
            })
            .await;

        assert!(recorder.pending_context.is_empty());
        assert_eq!(counters.analysis_requests_seen.load(Ordering::Relaxed), 1);
        assert_eq!(
            counters.analysis_requests_dropped.load(Ordering::Relaxed),
            1
        );
        assert_eq!(counters.correlation_degraded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn oversized_provider_raw_usage_is_omitted_at_the_ipc_boundary() {
        let padding = "x".repeat(12_000);
        let response_body = format!(
            "{{\"id\":\"resp_1\",\"model\":\"gpt-5.6-luna\",\"status\":\"completed\",\"usage\":{{\"input_tokens\":7,\"output_tokens\":3,\"total_tokens\":10,\"padding\":\"{padding}\"}}}}"
        );
        let limits = ObservationLimits::default();
        let request = parse_request(ObservationInput::new(
            br#"{"model":"gpt-5.6-luna","stream":true}"#,
            limits,
        ));
        let response = parse_response(ObservationInput::new(response_body.as_bytes(), limits));
        assert!(response.raw_usage.is_some());

        let ids = UuidV7Generator::new();
        let observation = ObservationRecord::new(42, request, "unix-ms:1")
            .with_response(response)
            .with_ended_at("unix-ms:2");
        let control_request = bounded_record_provider_observation_request(
            SessionId::generate(&ids),
            OperationId::generate(&ids),
            observation,
        );

        let frame_fits = serde_json::to_vec(&control_request)
            .ok()
            .and_then(|body| {
                super::MaxRequestBodyBytes::new(super::BODY_BYTES)
                    .ok()
                    .and_then(|maximum| {
                        super::IpcRequest::new(RequestId::generate(&ids), body, maximum).ok()
                    })
            })
            .and_then(|request| serde_json::to_vec(&request).ok())
            .map(|frame| {
                u64::try_from(frame.len()).is_ok_and(|length| length <= super::FRAME_BYTES)
            });
        assert_eq!(frame_fits, Some(true));
        let retained = match &control_request {
            ControlRequest::RecordProviderObservation { observation, .. } => {
                observation.response.as_ref().map(|response| {
                    (
                        response.raw_usage.is_none(),
                        response
                            .normalized_usage
                            .as_ref()
                            .and_then(|usage| usage.input_total),
                    )
                })
            }
            _ => None,
        };
        assert_eq!(retained, Some((true, Some(7))));
    }

    #[test]
    fn context_lifecycle_bounds_reordered_terminal_holes() {
        let counters = ContextCounters::default();
        let total = u64::try_from(TERMINAL_ANALYSIS_IDENTITIES).unwrap_or(u64::MAX) + 32;
        for ordinal in 0..total {
            counters.ensure_seen_sequence(AnalysisSequence(ordinal));
        }
        for ordinal in (1..total).rev() {
            counters.complete_sequence(AnalysisSequence(ordinal));
        }

        let (terminal_through, terminal_holes_len) = {
            let lifecycle = counters
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (lifecycle.terminal_through, lifecycle.terminal_holes.len())
        };
        assert_eq!(terminal_through, None);
        assert_eq!(terminal_holes_len, TERMINAL_ANALYSIS_IDENTITIES);

        counters.complete_sequence(AnalysisSequence(0));
        counters.complete_sequence(AnalysisSequence(0));
        let (terminal_through, terminal_holes_len) = {
            let lifecycle = counters
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (lifecycle.terminal_through, lifecycle.terminal_holes.len())
        };
        assert_eq!(terminal_through, Some(AnalysisSequence(0)));
        assert!(terminal_holes_len <= TERMINAL_ANALYSIS_IDENTITIES);

        let seen = counters.analysis_requests_seen.load(Ordering::Relaxed);
        let complete = counters.analysis_requests_complete.load(Ordering::Relaxed);
        let partial = counters.analysis_requests_partial.load(Ordering::Relaxed);
        let dropped = counters.analysis_requests_dropped.load(Ordering::Relaxed);
        assert_eq!(seen, total);
        assert_eq!(seen, complete + partial + dropped);
    }

    #[tokio::test]
    async fn timeout_reconciliation_committed_statuses_preserve_terminal_partition() {
        let counters = ContextCounters::default();
        let ids = UuidV7Generator::new();

        for ordinal in 0..2 {
            let sequence = AnalysisSequence(ordinal);
            counters.ensure_seen_sequence(sequence);
            let evidence = PendingAnalysisEvidence {
                sequence,
                provider_request_id: RequestId::generate(&ids),
                snapshot_id: Some(ContextSnapshotId::generate(&ids)),
            };
            let mut lifecycle = counters
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _previous = lifecycle.evidence.insert(sequence, evidence);
        }

        reconcile_pending_context_with(&counters, |evidence| async move {
            let status = if evidence.sequence == AnalysisSequence(0) {
                "complete"
            } else {
                "partial"
            };
            evidence.snapshot_id.map(|snapshot_id| {
                ContextSnapshotStatus::new(
                    snapshot_id,
                    evidence.provider_request_id,
                    status.to_owned(),
                )
                .with_completed_at(Some(1))
            })
        })
        .await;

        assert_eq!(
            counters.analysis_requests_complete.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            counters.analysis_requests_partial.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            counters.analysis_requests_dropped.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            counters.analysis_requests_seen.load(Ordering::Relaxed),
            counters.analysis_requests_complete.load(Ordering::Relaxed)
                + counters.analysis_requests_partial.load(Ordering::Relaxed)
                + counters.analysis_requests_dropped.load(Ordering::Relaxed)
        );
        assert!(counters.pending_evidence().is_empty());
    }

    #[tokio::test]
    async fn timeout_reconciliation_active_or_unavailable_statuses_drop_exact_partition() {
        let counters = ContextCounters::default();
        let ids = UuidV7Generator::new();

        for ordinal in 0..3 {
            let sequence = AnalysisSequence(ordinal);
            counters.ensure_seen_sequence(sequence);
            let evidence = PendingAnalysisEvidence {
                sequence,
                provider_request_id: RequestId::generate(&ids),
                snapshot_id: Some(ContextSnapshotId::generate(&ids)),
            };
            let mut lifecycle = counters
                .lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _previous = lifecycle.evidence.insert(sequence, evidence);
        }

        reconcile_pending_context_with(&counters, |evidence| async move {
            if evidence.sequence == AnalysisSequence(0) {
                evidence.snapshot_id.map(|snapshot_id| {
                    ContextSnapshotStatus::new(
                        snapshot_id,
                        evidence.provider_request_id,
                        "active".to_owned(),
                    )
                })
            } else {
                // `ContextStatus` not-found and transport-unreachable both produce no status.
                None
            }
        })
        .await;

        assert_eq!(
            counters.analysis_requests_complete.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            counters.analysis_requests_partial.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            counters.analysis_requests_dropped.load(Ordering::Relaxed),
            0
        );
        assert_eq!(counters.pending_evidence().len(), 3);

        counters.drop_pending(ContextAnalysisDropReason::Cancelled);

        assert_eq!(
            counters.analysis_requests_dropped.load(Ordering::Relaxed),
            3
        );
        assert_eq!(counters.pending_drop_cancelled.load(Ordering::Relaxed), 3);
        assert_eq!(
            counters
                .pre_persistence_drop_cancelled
                .load(Ordering::Relaxed),
            3
        );
        assert_eq!(
            counters.analysis_requests_seen.load(Ordering::Relaxed),
            counters.analysis_requests_complete.load(Ordering::Relaxed)
                + counters.analysis_requests_partial.load(Ordering::Relaxed)
                + counters.analysis_requests_dropped.load(Ordering::Relaxed)
        );
        assert!(counters.pending_evidence().is_empty());
    }
    #[test]
    fn transport_admission_is_bounded_when_dispatcher_stalls() {
        use super::{TRANSPORT_DISPATCH_QUEUE_ITEMS, TransportOrdering, TransportSequence};

        let (sender, mut jobs) = tokio::sync::mpsc::channel(TRANSPORT_DISPATCH_QUEUE_ITEMS);
        let ordering = TransportOrdering::new(sender);
        for ordinal in 0..TRANSPORT_DISPATCH_QUEUE_ITEMS {
            assert!(ordering.try_enqueue_placeholder(TransportSequence(ordinal as u64)));
        }
        assert_eq!(ordering.pending_len(), TRANSPORT_DISPATCH_QUEUE_ITEMS);
        assert!(
            !ordering
                .try_enqueue_placeholder(TransportSequence(TRANSPORT_DISPATCH_QUEUE_ITEMS as u64))
        );
        assert_eq!(ordering.pending_len(), TRANSPORT_DISPATCH_QUEUE_ITEMS);

        let mut queued = 0;
        while jobs.try_recv().is_ok() {
            queued += 1;
        }
        assert_eq!(queued, TRANSPORT_DISPATCH_QUEUE_ITEMS);
    }

    #[test]
    fn transport_dispatcher_preserves_fifo_admission_order() {
        use super::{TransportDispatchJob, TransportOrdering, TransportSequence};

        let (sender, mut jobs) = tokio::sync::mpsc::channel(2);
        let ordering = TransportOrdering::new(sender);
        assert!(ordering.try_enqueue_placeholder(TransportSequence(11)));
        assert!(ordering.try_enqueue_placeholder(TransportSequence(12)));
        drop(ordering);

        assert!(matches!(
            jobs.try_recv(),
            Ok(TransportDispatchJob::Placeholder {
                sequence: TransportSequence(11),
                ..
            })
        ));
        assert!(matches!(
            jobs.try_recv(),
            Ok(TransportDispatchJob::Placeholder {
                sequence: TransportSequence(12),
                ..
            })
        ));
        assert!(jobs.try_recv().is_err());
    }
}
