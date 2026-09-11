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
    clippy::significant_drop_tightening,
    reason = "CLI boundary formats user-facing output and validates bounded fixed-size state"
)]
//! Tracepress command-line boundary.
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracepress_context::{
    ContextAnalysisLimits, ContextAnalysisResult, ContextAnalysisStatus, ContextBlockKind,
    ContextBlockSummary, ContextDeltaRequest, ContextOrigin, ContextRole, MeasurementApplicability,
    TokenEstimateAggregate, TokenReconciliation, compute_context_delta,
};
use tracepress_core::{
    AttemptId, ContextSnapshotId, HttpStatusCode, MaxIpcFrameBytes, MaxRequestBodyBytes,
    MaxResponseBodyBytes, OperationId, RequestId, ResourceLimits, ResourceLimitsConfig, SessionId,
    UuidV7Generator,
};
use tracepress_daemon::{
    ContextAnalysisFinalize, ContextAnalysisMetrics, ContextAppendReceipt,
    ContextCorrelationStatusWire, ControlRequest, ControlResponse, CorrelationDegradation,
    CorrelationStatus, ProviderObservation as ObservationRecord, ProviderObservationOutcome,
};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcLimits, IpcRequest, ResponseOutcome, UnixEndpoint,
};
use tracepress_provider::{
    ProviderEndpoint, ProviderRequestKind, ProviderResponseState, ProviderTransport,
    RequestObservation, ResponseObservation,
};
use tracepress_proxy::{
    CompactionObservation, ContextAnalysisDropReason, ContextAnalysisMode,
    ContextAnalysisObservation, ContextAnalysisOutcome, ForwardId, ForwardMetadata, InboundRoute,
    MetadataSink, MetadataSinkError, ObservationSinkError, ProviderObservationSink, ProxyConfig,
    RequestContextObservation, TransparentProxy, TransportFailure,
};
use tracepress_storage::{
    ContextInspection, ContextInspectionBlock, ContextInspectionNamedEstimate,
    ContextSnapshotStatus,
};

const FRAME_BYTES: u64 = 65_536;

/// Bounded queue of transport and semantic records awaiting durable recording.
const RECORDER_QUEUE_ITEMS: usize = 128;

/// Explicit cap for heavy context-ingestion jobs. One item may contain thousands of blocks, so
/// this queue is intentionally much smaller than the IPC item-count queue.
const CONTEXT_INGESTION_QUEUE_HARD_CAP: usize = 4;

/// Bounds heavy analyzed result payloads retained by the recorder and ingestion worker together.
const CONTEXT_HEAVY_OUTCOME_CAP: usize = CONTEXT_INGESTION_QUEUE_HARD_CAP;

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
        Option<OwnedSemaphorePermit>,
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
    analysis_permit: Option<OwnedSemaphorePermit>,
    provider_input_tokens: Option<u64>,
    provider_usage_comparable: bool,
    correlation: CorrelationStatus,
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
    analysis_permit: Option<OwnedSemaphorePermit>,
}
struct ContextFailureInput<'context> {
    forward: ForwardId,
    context: Option<&'context ContextAnalysisOutcome>,
    analysis_permit: Option<OwnedSemaphorePermit>,
}

struct EnqueueContextInput<'receipt> {
    receipt: &'receipt ContextReceipt,
    outcome: ContextAnalysisOutcome,
    analysis_permit: Option<OwnedSemaphorePermit>,
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
    analysis_permit: Option<OwnedSemaphorePermit>,
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

/// Accounting for context analyses rejected before context persistence.
#[derive(Debug, Default)]
struct ContextCounters {
    observer_backpressure: AtomicU64,
    resource_limit: AtomicU64,
    malformed: AtomicU64,
    correlation_degraded: AtomicU64,
    unsupported: AtomicU64,
    cancelled: AtomicU64,
    pre_persistence_drop_backpressure: AtomicU64,
    pre_persistence_drop_resource_limit: AtomicU64,
    pre_persistence_drop_malformed: AtomicU64,
    pre_persistence_drop_correlation: AtomicU64,
    pre_persistence_drop_unsupported: AtomicU64,
    pre_persistence_drop_cancelled: AtomicU64,
    pending_drop_backpressure: AtomicU64,
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
            "context_observer_backpressure_total={}\ncontext_resource_limit_total={}\ncontext_malformed_total={}\ncontext_correlation_degraded_total={}\ncontext_unsupported_total={}\ncontext_cancelled_total={}\nanalysis_requests_seen={seen}\nanalysis_requests_complete={complete}\nanalysis_requests_partial={partial}\nanalysis_requests_dropped={dropped}\nanalysis_drop_backpressure={}\nanalysis_drop_resource_limit={}\nanalysis_drop_malformed={}\nanalysis_drop_correlation={}\nanalysis_drop_unsupported={}\nanalysis_drop_cancelled={}\ncorrelation_eligible={correlation_eligible}\ncorrelation_correlated={correlation_correlated}\nanalysis_coverage={analysis_coverage}\ncorrelation_coverage={correlation_coverage}\nanalysis_coverage_complete={complete_coverage}\nanalysis_coverage_partial={partial_coverage}\nanalysis_coverage_dropped={dropped_coverage}\ntoken_estimation_coverage={token_coverage}\nsemantic_detection_coverage={semantic_coverage}",
            self.observer_backpressure.load(Ordering::Relaxed),
            self.resource_limit.load(Ordering::Relaxed),
            self.malformed.load(Ordering::Relaxed),
            self.correlation_degraded.load(Ordering::Relaxed),
            self.unsupported.load(Ordering::Relaxed),
            self.cancelled.load(Ordering::Relaxed),
            self.pre_persistence_drop_backpressure
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
    sender: tokio::sync::mpsc::Sender<RunEvent>,
    ordering: Arc<TransportOrdering>,
    counters: Arc<CorrelationCounters>,
    context_counters: Arc<ContextCounters>,
    analysis_slots: Arc<Semaphore>,
    analysis_enabled: bool,
}

impl RecorderSink {
    fn offer(&self, event: RunEvent) -> Result<(), MetadataSinkError> {
        self.sender
            .try_send(event)
            .map_err(|_error| MetadataSinkError::rejected())
    }

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
            .sender
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
                    .sender
                    .try_send(RunEvent::TransportAdmissionFailed(forward, reason));
                self.sender
                    .try_send(RunEvent::Response(
                        forward,
                        failure.observation,
                        CorrelationStatus::Degraded(reason),
                    ))
                    .map_err(|_error| MetadataSinkError::rejected())
            }
        }
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
        match self.offer_durable(RunEvent::RequestContext(Box::new(observation))) {
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
            ContextAnalysisOutcome::Analyzed(_) => {
                match Arc::clone(&self.analysis_slots).try_acquire_owned() {
                    Ok(permit) => Some(permit),
                    Err(_error) => {
                        if self.analysis_enabled {
                            self.context_counters
                                .dropped(forward, ContextAnalysisDropReason::ObserverBackpressure);
                        }
                        return Err(ObservationSinkError::rejected());
                    }
                }
            }
            _ => None,
        };
        match self.offer(RunEvent::ContextAnalysis(
            observation.forward,
            observation.outcome,
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
    analysis_permit: Option<OwnedSemaphorePermit>,
    /// A Phase 2 request event arrived before its detached Phase 3 outcome.
    context_pending: bool,
    status_code: Option<u16>,
    transport_failure: Option<TransportFailure>,
    /// Transport admission failed before status could be joined to this forward.
    transport_admission_failure: Option<CorrelationDegradation>,
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
        let analysis_permit = self.analysis_permit.take();
        let transport_failure = self.transport_failure.take();
        let orphaned_semantic =
            self.request.is_none() && (response.is_some() || transport_failure.is_some());
        ForwardEvidence {
            forward,
            semantic: self.request.take().map(|pending| SemanticRecord {
                pending,
                response,
                context,
                analysis_permit,
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
    analysis_permit: Option<OwnedSemaphorePermit>,
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
                RunEvent::ContextAnalysis(forward, outcome, analysis_permit) => {
                    self.context_analysis(ContextAnalysisInput {
                        forward,
                        outcome,
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
            ProviderRequestKind::Compaction,
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
            ControlRequest::RecordProviderObservation {
                session_id: self.session_id,
                parent_operation_id: self.parent_operation_id,
                observation: Box::new(record),
            },
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

    /// Buffers the Phase 2 request half before dispatch and waits for its detached Phase 3 outcome
    /// before checking terminal evidence.
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
            state.context = observation.context;
            state.context_pending = state.context.is_none();
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
        state.context = observation.context;
        state.context_pending = state.context.is_none();
        if state.response.is_none() && state.transport_failure.is_none() {
            return;
        }
        state.settled = true;
        let mut evidence = Some(state.evidence(forward));
        if let Some(evidence) = evidence.take() {
            let record = self.record(evidence, CorrelationStatus::Correlated);
            record.await;
        }
    }

    /// Attaches the detached Phase 3 outcome after the forwarding task has crossed dispatch.
    async fn context_analysis(&mut self, input: ContextAnalysisInput) {
        let ContextAnalysisInput {
            forward,
            outcome,
            analysis_permit,
        } = input;
        if self.analysis_enabled {
            self.context_counters.ensure_seen(forward);
        }
        if let Some(receipt) = self.pending_context.remove(&forward) {
            self.enqueue_context(EnqueueContextInput {
                receipt: &receipt,
                outcome,
                analysis_permit,
            });
            return;
        }
        let Some(state) = self.forwards.get_mut(&forward) else {
            self.context_counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return;
        };
        if state.settled {
            self.context_counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return;
        }
        state.context = Some(outcome);
        state.analysis_permit = analysis_permit;
        state.context_pending = false;
        if state.request.is_none() || state.response.is_none() && state.transport_failure.is_none()
        {
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
        let permit = state.analysis_permit.take();
        let Some(outcome) = state.context.take() else {
            drop(permit);
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
        drop(permit);
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
        .await
        .map_err(|_error| ContextAnalysisDropReason::CorrelationDegraded)?;
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
            analysis_permit,
            provider_input_tokens,
            provider_usage_comparable,
            correlation,
        } = job;
        let (snapshot_id, started_at_us) = match self
            .begin_context(provider_request_id, inference_operation_id)
            .await
        {
            Ok(value) => value,
            Err(reason) => {
                drop(analysis_permit);
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
                drop(analysis_permit);
                self.counters.dropped(forward, reason);
                return;
            }
        };
        self.finalize_context(ContextFinalizationInput {
            forward,
            snapshot_id,
            started_at_us,
            attempt_id,
            analysis: &analysis,
            analysis_permit,
            provider_input_tokens,
            provider_usage_comparable,
            correlation,
            status,
            accepted_block_count,
        })
        .await;
    }

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
        let (metrics, visible_estimated_tokens) = context_metrics(
            accepted_blocks,
            input.status,
            delta
                .as_ref()
                .and_then(|value| value.common_prefix_estimated_tokens),
        );
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
    async fn finalize_context(&mut self, input: ContextFinalizationInput<'_>) {
        let forward = input.forward;
        let snapshot_id = input.snapshot_id;
        let status = input.status;
        let Some((current_blocks, finalize, coverage)) = self.build_context_finalize(&input) else {
            self.abort_context(snapshot_id, ContextAnalysisDropReason::CorrelationDegraded)
                .await;
            drop(input.analysis_permit);
            self.counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return;
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
            }
        }
        drop(input.analysis_permit);
    }
}
async fn flush_context_drops(config: &Config, session_id: SessionId, counters: &ContextCounters) {
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
    Option<OwnedSemaphorePermit>,
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
        analysis_permit,
        status_code,
        transport_failure,
        ..
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
    previous_context: Option<PreviousContextSnapshot>,
}
/// The recorder of one run: its proxy, its workers, and the counters it publishes.
struct SpawnedRecorder {
    proxy: TransparentProxy,
    recorder_task: tokio::task::JoinHandle<Result<(), String>>,
    transport_task: tokio::task::JoinHandle<()>,
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
        analysis_enabled,
        previous_context,
    } = recording;
    let (sender, receiver) = tokio::sync::mpsc::channel(RECORDER_QUEUE_ITEMS);
    let (transport_sender, transport_receiver) =
        tokio::sync::mpsc::channel(TRANSPORT_DISPATCH_QUEUE_ITEMS);
    let context_queue_items = context_ingestion_queue_capacity(context_queue_items);
    let (context_sender, context_receiver) = tokio::sync::mpsc::channel(context_queue_items);
    let counters = Arc::new(CorrelationCounters::default());
    let context_counters = Arc::new(ContextCounters::default());
    let analysis_slots = Arc::new(Semaphore::new(CONTEXT_HEAVY_OUTCOME_CAP));
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
    let ordering = Arc::new(TransportOrdering::new(transport_sender));
    let proxy = proxy
        .with_metadata_sink(Arc::new(RecorderSink {
            sender: sender.clone(),
            ordering: Arc::clone(&ordering),
            counters: Arc::clone(&counters),
            context_counters: Arc::clone(&context_counters),
            analysis_slots: Arc::clone(&analysis_slots),
            analysis_enabled,
        }))
        .with_observation_sink(Arc::new(RecorderSink {
            sender,
            ordering,
            counters: Arc::clone(&counters),
            context_counters: Arc::clone(&context_counters),
            analysis_slots,
            analysis_enabled,
        }));
    SpawnedRecorder {
        proxy,
        recorder_task,
        transport_task,
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

fn configure_codex_subscription(command: &mut Command, proxy_address: std::net::SocketAddr) {
    let base_url = format!("http://{proxy_address}/v1");
    let _command = command.args([
        "-c",
        "model_provider=tracepress_subscription",
        "-c",
        "model_providers.tracepress_subscription.name=OpenAI",
        "-c",
        &format!("model_providers.tracepress_subscription.base_url=\"{base_url}\""),
        "-c",
        "model_providers.tracepress_subscription.wire_api=\"responses\"",
        "-c",
        "model_providers.tracepress_subscription.requires_openai_auth=true",
        "-c",
        "model_providers.tracepress_subscription.supports_websockets=false",
    ]);
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
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    };
    let SpawnedRecorder {
        proxy,
        mut recorder_task,
        mut transport_task,
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
            analysis_enabled: matches!(analysis_mode, ContextAnalysisMode::Shadow),
            previous_context: None,
        },
        proxy,
    );
    let background_proxy = proxy.clone();
    let proxy_task = tokio::spawn(async move { serve(listener, proxy.router()).await });
    let base_url = format!("http://{proxy_address}/v1");
    let mut command = Command::new(&agent);
    if matches!(transport, ProviderTransport::ChatGptCodexSubscription) && is_codex_agent(&agent) {
        configure_codex_subscription(&mut command, proxy_address);
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
    let recorded = match tokio::time::timeout(RECORDER_DRAIN_TIMEOUT, async {
        background_proxy.wait_for_background_tasks().await;
        // The recorder's channel must be closed before it is drained; otherwise the recorder
        // cannot observe end-of-run and wait for more events forever.
        drop(background_proxy);
        drain_recorders(&mut recorder_task, &mut transport_task, &mut context_task).await
    })
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            // Reconcile any snapshot that committed before its response was lost. Only unresolved
            // identities are dropped at the cancellation boundary below.
            reconcile_pending_context(config, &context_counters).await;
            // The drain timeout is a terminal cancellation boundary. Account every admitted
            // analysis before aborting workers, then make one bounded best-effort drop flush.
            context_counters.drop_pending(ContextAnalysisDropReason::Cancelled);
            transport_task.abort();
            recorder_task.abort();
            context_task.abort();
            let _ = (&mut transport_task).await;
            let _ = (&mut recorder_task).await;
            let _ = (&mut context_task).await;
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                flush_context_drops(config, session.session_id, &context_counters),
            )
            .await;
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

async fn drain_recorders(
    recorder_task: &mut tokio::task::JoinHandle<Result<(), String>>,
    transport_task: &mut tokio::task::JoinHandle<()>,
    context_task: &mut tokio::task::JoinHandle<Result<(), String>>,
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
    let transport = std::env::var("TRACEPRESS_PROVIDER_TRANSPORT")
        .map_or(Ok(ProviderTransport::OpenAiPublicApi), |value| {
            provider_transport_from_value(&value)
        })?;
    let endpoint = provider_endpoint(transport)?;
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
    use std::sync::atomic::Ordering;

    use super::{
        AnalysisSequence, CONTEXT_INGESTION_QUEUE_HARD_CAP, ContextAnalysisDropReason,
        ContextCounters, ContextSnapshotId, ContextSnapshotStatus, PendingAnalysisEvidence,
        RequestId, TERMINAL_ANALYSIS_IDENTITIES, UuidV7Generator, context_ingestion_queue_capacity,
        reconcile_pending_context_with,
    };

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
