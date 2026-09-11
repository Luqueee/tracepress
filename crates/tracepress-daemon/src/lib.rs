#![allow(
    clippy::multiple_crate_versions,
    reason = "daemon dependencies currently select distinct platform support versions"
)]

//! Daemon-owned session and causal-operation lifecycle service.

use std::collections::HashMap;

pub(crate) mod context;

pub use context::{
    ContextAnalysisAbort, ContextAnalysisBegin, ContextAnalysisFinalize, ContextAnalysisMetrics,
    ContextAppendCapacity, ContextAppendReceipt, ContextBlockBatch, ContextCorrelationStatusWire,
};
use thiserror::Error;
use tokio::sync::Mutex;
use tracepress_context::{
    ContextAnalysisDropReason, ContextAnalysisStatus as AnalyzerAnalysisStatus,
};
use tracepress_core::{
    AttemptId, CausalEdge, CausalEdgeError, CausalRelationship, ContextBlockOccurrenceId,
    ContextSnapshotId, EventId, HttpStatusCode, InferenceStatus, OperationId, OperationKind,
    OperationStatus, RequestId, RequestMetadata, SessionId, SessionState, UsageStatus,
    UuidV7Generator,
};
use tracepress_provider::{
    AnalysisDecodeStatus as ProviderAnalysisDecodeStatus, AnomalyFlags,
    ContentEncoding as ProviderContentEncoding, ObservationStatus as ProviderObservationStatus,
    ProviderKind as CanonicalProviderKind, ProviderProtocol as CanonicalProviderProtocol,
    ProviderRequestKind, ProviderResponseState, ProviderTransport as CanonicalProviderTransport,
    RequestObservation, ResponseObservation, UsageStatus as ProviderUsageStatus,
};
use tracepress_storage::{
    AnalysisDecodeStatus, ContentEncoding, ContextInspection, ContextSnapshotStatus,
    ObservationStatus, ProviderKind, ProviderProtocol,
    ProviderResponseState as StorageResponseState, ProviderTransport, StorageError, StorageWriter,
    WriteBatch, WriteCommand, WriteReceipt,
};

/// Failure returned by the daemon lifecycle boundary.
#[allow(
    missing_docs,
    reason = "variant fields repeat the typed identities and states named by each error"
)]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DaemonError {
    /// Durable state could not be committed by the sole writer.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The requested session is unknown to this daemon instance.
    #[error("session {session_id} is not registered")]
    UnknownSession { session_id: SessionId },
    /// The requested operation is unknown or belongs to another session.
    #[error("operation {operation_id} is not registered in session {session_id}")]
    UnknownOperation {
        session_id: SessionId,
        operation_id: OperationId,
    },
    /// The requested operation is not a model-inference operation.
    #[error("operation {operation_id} in session {session_id} is not an LLM inference")]
    OperationNotInference {
        session_id: SessionId,
        operation_id: OperationId,
        kind: OperationKind,
    },
    /// A non-active session cannot accept another operation.
    #[error("session {session_id} in state {state:?} cannot accept operations")]
    SessionNotActive {
        session_id: SessionId,
        state: SessionState,
    },
    /// The requested context snapshot is unknown or has already been finalized.
    #[error("context snapshot {snapshot_id} is not active")]
    UnknownContextSnapshot {
        snapshot_id: tracepress_core::ContextSnapshotId,
    },
    /// The requested provider request has no finalized context snapshot.
    #[error("context inspection request {request_id} was not found")]
    ContextInspectionNotFound { request_id: RequestId },
    /// A context analysis identity does not match its provider request and inference operation.
    #[error("context analysis identities are not associated with session {session_id}")]
    InvalidContextAssociation { session_id: SessionId },
    /// An append sequence was not the next expected sequence.
    #[error("context snapshot {snapshot_id} expected sequence {expected}, received {actual}")]
    ContextSequence {
        snapshot_id: tracepress_core::ContextSnapshotId,
        expected: u32,
        actual: u32,
    },
    /// The bounded active-analysis table is full; no snapshot was admitted.
    #[error("context analysis capacity exhausted (limit {limit})")]
    ContextAnalysisCapacity { limit: usize },
    /// A context append exceeded one of the bounded analysis dimensions.
    #[error("context snapshot {snapshot_id} exceeded {dimension}")]
    ContextLimit {
        snapshot_id: tracepress_core::ContextSnapshotId,
        dimension: &'static str,
    },
    /// A context block was not in the required document order or had an unknown parent.
    #[error("context snapshot {snapshot_id} has invalid block ordinal or parent")]
    InvalidContextBlock {
        snapshot_id: tracepress_core::ContextSnapshotId,
    },
    /// A context snapshot was finalized more than once.
    #[error("context snapshot {snapshot_id} was already finalized")]
    ContextAlreadyFinalized {
        snapshot_id: tracepress_core::ContextSnapshotId,
    },
    /// A finalize payload referred to a different snapshot.
    #[error("context finalize payload does not match snapshot {snapshot_id}")]
    ContextFinalizeMismatch {
        snapshot_id: tracepress_core::ContextSnapshotId,
    },
    /// A context finalize payload was not internally consistent.
    #[error("context finalize payload for snapshot {snapshot_id} is invalid")]
    InvalidContextFinalize {
        snapshot_id: tracepress_core::ContextSnapshotId,
    },
    /// The requested causal edge violates the core DAG contract.
    #[error(transparent)]
    InvalidCausalEdge(#[from] CausalEdgeError),
}

/// Read-only lifecycle view returned to daemon consumers.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct SessionSnapshot {
    /// Session identity.
    pub session_id: SessionId,
    /// Current lifecycle state.
    pub state: SessionState,
    /// Session-specific ingress key.
    pub ingress_key: String,
    /// Operations currently registered in this session.
    pub operations: usize,
}
#[derive(Debug)]
struct SessionRecord {
    state: SessionState,
    ingress_key: String,
    operations: HashMap<OperationId, OperationKind>,
    provider_requests: HashMap<OperationId, RequestId>,
    next_attempt_ordinals: HashMap<OperationId, u64>,
}

/// Transport evidence used when a provider response is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderObservationOutcome {
    /// The attempt is still in progress or has no terminal transport evidence.
    InProgress,
    /// The provider completed successfully.
    Completed,
    /// The provider ended without complete output.
    Incomplete,
    /// The client cancelled the attempt.
    Cancelled,
    /// The upstream disconnected before a terminal response.
    Disconnected,
    /// The transport or provider reported an error.
    Failed,
}

/// Why one forward's correlation evidence could not be completed.
///
/// The recorder joins the two halves of a forward inside bounded state. Each reason names one
/// specific way that bounded state lost evidence, so a degraded dataset stays attributable
/// instead of merely suspicious.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CorrelationDegradation {
    /// Correlation state was evicted before settling because the in-flight bound was reached.
    InFlightLimit,
    /// A half arrived for an identity the bounded retirement memory still refuses to re-admit.
    RetiredLimit,
    /// Terminal semantic evidence arrived for a forward whose request half never did.
    MissingRequestHalf,
}

/// Whether one recorded forward's correlation evidence is complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CorrelationStatus {
    /// Both halves of the forward were joined by its correlation identity.
    Correlated,
    /// Correlation was incomplete: the record carries evidence, never complete causality.
    Degraded(CorrelationDegradation),
}

/// Canonical provider observations plus bounded transport evidence.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ProviderObservation {
    /// Exact accepted request body length.
    pub request_bytes: u64,
    /// Semantic request observation produced by `tracepress-provider`.
    pub request: RequestObservation,
    /// Semantic response observation, when response bytes were observed.
    pub response: Option<ResponseObservation>,
    /// Daemon timestamp at which the attempt started.
    pub started_at: String,
    /// Daemon timestamp at which the attempt ended, if terminal.
    pub ended_at: Option<String>,
    /// Upstream HTTP status, when response headers were observed.
    pub status_code: Option<HttpStatusCode>,
    /// Exact response bytes forwarded for transport-only observations.
    #[serde(default)]
    pub response_bytes: Option<u64>,
    /// Monotonic transport duration for transport-only observations.
    #[serde(default)]
    pub duration_us: Option<u64>,
    /// Transport error text, when forwarding failed.
    pub transport_error: Option<String>,
    /// Explicit transport lifecycle evidence for response-less attempts.
    pub outcome: ProviderObservationOutcome,
    /// Whether the response was streamed.
    pub streaming: Option<bool>,
    /// Whether the recorder could join both halves of this forward.
    pub correlation_status: CorrelationStatus,
}

impl ProviderObservation {
    /// Creates a provider observation for one logical request attempt.
    #[must_use]
    pub fn new(
        request_bytes: u64,
        request: RequestObservation,
        started_at: impl Into<String>,
    ) -> Self {
        Self {
            request_bytes,
            request,
            response: None,
            started_at: started_at.into(),
            ended_at: None,
            status_code: None,
            response_bytes: None,
            duration_us: None,
            transport_error: None,
            outcome: ProviderObservationOutcome::InProgress,
            streaming: None,
            correlation_status: CorrelationStatus::Correlated,
        }
    }

    /// Adds semantic response evidence.
    #[must_use]
    pub fn with_response(mut self, response: ResponseObservation) -> Self {
        self.response = Some(response);
        self
    }
    /// Adds terminal timestamp evidence.
    #[must_use]
    pub fn with_ended_at(mut self, ended_at: impl Into<String>) -> Self {
        self.ended_at = Some(ended_at.into());
        self
    }

    /// Adds an observed upstream HTTP status.
    #[must_use]
    pub const fn with_status_code(mut self, status_code: HttpStatusCode) -> Self {
        self.status_code = Some(status_code);
        self
    }

    /// Adds bounded transport byte and duration measurements.
    #[must_use]
    pub const fn with_transport_metrics(
        mut self,
        response_bytes: u64,
        duration_us: Option<u64>,
    ) -> Self {
        self.response_bytes = Some(response_bytes);
        self.duration_us = duration_us;
        self
    }

    /// Adds a transport error and marks the attempt failed.
    #[must_use]
    pub fn with_transport_error(mut self, transport_error: impl Into<String>) -> Self {
        self.transport_error = Some(transport_error.into());
        self.outcome = ProviderObservationOutcome::Failed;
        self
    }

    /// Adds explicit response-less lifecycle evidence.
    #[must_use]
    pub const fn with_outcome(mut self, outcome: ProviderObservationOutcome) -> Self {
        self.outcome = outcome;
        self
    }

    /// Sets whether the response was streamed.
    #[must_use]
    pub const fn with_streaming(mut self, streaming: bool) -> Self {
        self.streaming = Some(streaming);
        self
    }

    /// Records how completely the recorder could correlate this forward.
    #[must_use]
    pub const fn with_correlation_status(mut self, correlation_status: CorrelationStatus) -> Self {
        self.correlation_status = correlation_status;
        self
    }
}

/// All evidence and identities required to persist one provider observation.
#[derive(Clone, Debug)]
pub struct PersistProviderObservation {
    session_id: SessionId,
    operation_id: OperationId,
    observation: ProviderObservation,
}

impl PersistProviderObservation {
    /// Combines the inference identity and its observed provider evidence.
    #[must_use]
    pub const fn new(
        session_id: SessionId,
        operation_id: OperationId,
        observation: ProviderObservation,
    ) -> Self {
        Self {
            session_id,
            operation_id,
            observation,
        }
    }
}

/// All identities and evidence required to record one provider observation.
#[derive(Clone, Debug)]
pub struct RecordProviderObservation {
    session_id: SessionId,
    parent_operation_id: OperationId,
    observation: ProviderObservation,
}

impl RecordProviderObservation {
    /// Combines the caller's root operation and its observed provider evidence.
    #[must_use]
    pub const fn new(
        session_id: SessionId,
        parent_operation_id: OperationId,
        observation: ProviderObservation,
    ) -> Self {
        Self {
            session_id,
            parent_operation_id,
            observation,
        }
    }
}

/// All identities and evidence required to record one correlation degradation.
#[derive(Clone, Debug)]
pub struct RecordCorrelationDegradation {
    session_id: SessionId,
    reason: CorrelationDegradation,
    observed_at: String,
}

impl RecordCorrelationDegradation {
    /// Combines the degraded session with the reason and the moment it was observed.
    #[must_use]
    pub const fn new(
        session_id: SessionId,
        reason: CorrelationDegradation,
        observed_at: String,
    ) -> Self {
        Self {
            session_id,
            reason,
            observed_at,
        }
    }
}

/// Missing required input while constructing a context-analysis drop report.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum RecordContextAnalysisDroppedBuildError {
    /// A required builder field was not supplied.
    #[error("context analysis drop builder is missing `{0}`")]
    MissingField(&'static str),
}

/// Builder for a bounded count of context analyses rejected before context persistence.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RecordContextAnalysisDroppedBuilder {
    session_id: Option<SessionId>,
    reason: Option<ContextAnalysisDropReason>,
    dropped: Option<u64>,
    observed_at_us: Option<u64>,
}

/// A bounded count of context analyses rejected before context persistence.
#[derive(Clone, Debug)]
pub struct RecordContextAnalysisDropped {
    session_id: SessionId,
    reason: ContextAnalysisDropReason,
    dropped: u64,
    observed_at_us: u64,
}

impl RecordContextAnalysisDropped {
    /// Starts building a compact context-analysis drop report.
    #[must_use]
    pub const fn builder() -> RecordContextAnalysisDroppedBuilder {
        RecordContextAnalysisDroppedBuilder {
            session_id: None,
            reason: None,
            dropped: None,
            observed_at_us: None,
        }
    }
}

impl RecordContextAnalysisDroppedBuilder {
    /// Sets the active session identity.
    #[must_use]
    pub const fn session_id(mut self, value: SessionId) -> Self {
        self.session_id = Some(value);
        self
    }

    /// Sets the reason analyses were dropped.
    #[must_use]
    pub const fn reason(mut self, value: ContextAnalysisDropReason) -> Self {
        self.reason = Some(value);
        self
    }

    /// Sets the number of analyses dropped.
    #[must_use]
    pub const fn dropped(mut self, value: u64) -> Self {
        self.dropped = Some(value);
        self
    }

    /// Sets the observation timestamp in microseconds.
    #[must_use]
    pub const fn observed_at_us(mut self, value: u64) -> Self {
        self.observed_at_us = Some(value);
        self
    }

    /// Completes the context-analysis drop report after all fields are supplied.
    ///
    /// # Errors
    /// Returns the missing required field when the builder is incomplete.
    pub const fn build(
        self,
    ) -> Result<RecordContextAnalysisDropped, RecordContextAnalysisDroppedBuildError> {
        let Some(session_id) = self.session_id else {
            return Err(RecordContextAnalysisDroppedBuildError::MissingField(
                "session_id",
            ));
        };
        let Some(reason) = self.reason else {
            return Err(RecordContextAnalysisDroppedBuildError::MissingField(
                "reason",
            ));
        };
        let Some(dropped) = self.dropped else {
            return Err(RecordContextAnalysisDroppedBuildError::MissingField(
                "dropped",
            ));
        };
        let Some(observed_at_us) = self.observed_at_us else {
            return Err(RecordContextAnalysisDroppedBuildError::MissingField(
                "observed_at_us",
            ));
        };
        Ok(RecordContextAnalysisDropped {
            session_id,
            reason,
            dropped,
            observed_at_us,
        })
    }
}

/// Identities assigned to a persisted provider observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ProviderObservationReceipt {
    /// Logical request identity shared by all retries.
    pub request_id: RequestId,
    /// Identity of this attempt.
    pub attempt_id: AttemptId,
    /// Zero-based ordinal of this attempt for the logical request.
    pub ordinal: u64,
}

impl ProviderObservationReceipt {
    /// Returns the provider request identity using the Phase 2 correlation vocabulary.
    #[must_use]
    pub const fn provider_request_id(self) -> RequestId {
        self.request_id
    }
}

impl RecordedProviderObservation {
    /// Returns the inference operation identity using the Phase 2 correlation vocabulary.
    #[must_use]
    pub const fn inference_operation_id(self) -> OperationId {
        self.operation_id
    }

    /// Returns the provider request identity using the Phase 2 correlation vocabulary.
    #[must_use]
    pub const fn provider_request_id(self) -> RequestId {
        self.receipt.request_id
    }
}

/// Identities assigned to one observation recorded beneath a root operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RecordedProviderObservation {
    /// Inference operation created beneath the caller's root operation.
    pub operation_id: OperationId,
    /// Durable provider request and attempt identities.
    pub receipt: ProviderObservationReceipt,
}

#[derive(Debug)]
struct ActiveContextAnalysis {
    session_id: SessionId,
    provider_request_id: RequestId,
    inference_operation_id: OperationId,
    analysis_version: u32,
    started_at_us: u64,
    next_sequence: u32,
    next_ordinal: u32,
    occurrences: HashMap<u32, ContextBlockOccurrenceId>,
}

#[derive(Clone, Copy)]
struct ContextSnapshotOutcomeInput {
    snapshot_id: ContextSnapshotId,
    status: AnalyzerAnalysisStatus,
    completed_at_us: u64,
    explicit_block_count: Option<u64>,
}

const fn context_snapshot_outcome(input: ContextSnapshotOutcomeInput) -> WriteCommand {
    let ContextSnapshotOutcomeInput {
        snapshot_id,
        status,
        completed_at_us,
        explicit_block_count,
    } = input;
    WriteCommand::ContextSnapshotOutcome {
        snapshot_id,
        status: map_analysis_status(status),
        completed_at_us: Some(completed_at_us),
        request_content_hash: None,
        analysis_content_hash: None,
        explicit_block_count,
        analyzed_bytes: None,
        skipped_bytes: None,
        explicit_request_complete: None,
        uses_previous_response: None,
        uses_conversation_state: None,
        uses_item_references: None,
        uses_prompt_reference: None,
        uses_external_files: None,
        uses_external_images: None,
        contains_opaque_items: None,
        logical_context_status: None,
        duplicate_key_detected: None,
        reference_resolved_locally: None,
        correlation_status: None,
    }
}

#[derive(Clone, Copy)]
struct ContextAnalysisDropPayloadInput {
    session_id: SessionId,
    reason: ContextAnalysisDropReason,
    dropped_count: u64,
    snapshot: Option<(ContextSnapshotId, u32, u32)>,
}

fn render_context_analysis_drop_payload(input: ContextAnalysisDropPayloadInput) -> Box<[u8]> {
    let ContextAnalysisDropPayloadInput {
        session_id,
        reason,
        dropped_count,
        snapshot,
    } = input;
    let status = reason.as_wire_str();
    let snapshot_fields = snapshot.map_or_else(String::new, |(
        snapshot_id,
        analysis_version,
        explicit_block_count,
    )| {
        format!(
            r#","snapshot_id":"{snapshot_id}","analysis_version":{analysis_version},"explicit_block_count":{explicit_block_count}"#
        )
    });
    format!(
        r#"{{"session_id":"{session_id}","status":"{status}","reason":"{status}","dropped_count":{dropped_count}{snapshot_fields}}}"#
    )
    .into_bytes()
    .into_boxed_slice()
}

fn context_analysis_drop_payload(
    session_id: SessionId,
    reason: ContextAnalysisDropReason,
    dropped_count: u64,
) -> Box<[u8]> {
    render_context_analysis_drop_payload(ContextAnalysisDropPayloadInput {
        session_id,
        reason,
        dropped_count,
        snapshot: None,
    })
}

fn context_analysis_dropped_event(
    active: &ActiveContextAnalysis,
    request: &ContextAnalysisAbort,
    ids: &UuidV7Generator,
) -> WriteCommand {
    let payload = render_context_analysis_drop_payload(ContextAnalysisDropPayloadInput {
        session_id: active.session_id,
        reason: request.reason,
        dropped_count: 1,
        snapshot: Some((
            request.snapshot_id,
            active.analysis_version,
            active.next_ordinal,
        )),
    });
    WriteCommand::Event {
        event_id: EventId::generate(ids),
        session_id: Some(active.session_id),
        operation_id: Some(active.inference_operation_id),
        timestamp: request.completed_at_us.to_string(),
        event_type: "context.analysis.dropped".to_owned(),
        payload,
        schema_version: "1".to_owned(),
    }
}

const fn map_analysis_status(
    value: AnalyzerAnalysisStatus,
) -> tracepress_storage::ContextAnalysisStatus {
    match value {
        AnalyzerAnalysisStatus::Complete => tracepress_storage::ContextAnalysisStatus::Complete,
        AnalyzerAnalysisStatus::ResourceLimit => {
            tracepress_storage::ContextAnalysisStatus::ResourceLimit
        }
        AnalyzerAnalysisStatus::Malformed => tracepress_storage::ContextAnalysisStatus::Malformed,
        AnalyzerAnalysisStatus::ObserverBackpressure => {
            tracepress_storage::ContextAnalysisStatus::ObserverBackpressure
        }
        AnalyzerAnalysisStatus::CorrelationDegraded => {
            tracepress_storage::ContextAnalysisStatus::CorrelationDegraded
        }
        AnalyzerAnalysisStatus::Unsupported => {
            tracepress_storage::ContextAnalysisStatus::Unsupported
        }
        AnalyzerAnalysisStatus::Cancelled => tracepress_storage::ContextAnalysisStatus::Cancelled,
        _ => tracepress_storage::ContextAnalysisStatus::Partial,
    }
}

const fn drop_analysis_status(reason: ContextAnalysisDropReason) -> AnalyzerAnalysisStatus {
    match reason {
        ContextAnalysisDropReason::ObserverBackpressure => {
            AnalyzerAnalysisStatus::ObserverBackpressure
        }
        ContextAnalysisDropReason::ResourceLimit => AnalyzerAnalysisStatus::ResourceLimit,
        ContextAnalysisDropReason::Malformed => AnalyzerAnalysisStatus::Malformed,
        ContextAnalysisDropReason::CorrelationDegraded => {
            AnalyzerAnalysisStatus::CorrelationDegraded
        }
        ContextAnalysisDropReason::Unsupported => AnalyzerAnalysisStatus::Unsupported,
        ContextAnalysisDropReason::Cancelled => AnalyzerAnalysisStatus::Cancelled,
        _ => AnalyzerAnalysisStatus::Partial,
    }
}

fn context_analysis_abort_batch(
    active: &ActiveContextAnalysis,
    request: &ContextAnalysisAbort,
    ids: &UuidV7Generator,
) -> WriteBatch {
    WriteBatch::new(context_snapshot_outcome(ContextSnapshotOutcomeInput {
        snapshot_id: request.snapshot_id,
        status: drop_analysis_status(request.reason),
        completed_at_us: request.completed_at_us,
        explicit_block_count: Some(u64::from(active.next_ordinal)),
    }))
    .and(context_analysis_dropped_event(active, request, ids))
}

#[derive(Clone, Copy)]
struct ContextAnalysisFinishInput<'analyses> {
    analyses: &'analyses HashMap<ContextSnapshotId, ActiveContextAnalysis>,
    session_id: SessionId,
    completed_at_us: u64,
    ids: &'analyses UuidV7Generator,
}

#[derive(Clone, Copy)]
struct ContextAnalysisPartialEventInput<'active, 'ids> {
    active: &'active ActiveContextAnalysis,
    snapshot_id: ContextSnapshotId,
    completed_at_us: u64,
    ids: &'ids UuidV7Generator,
}

fn context_analysis_finish_batch(
    input: ContextAnalysisFinishInput<'_>,
) -> (Option<WriteBatch>, Vec<ContextSnapshotId>) {
    let ContextAnalysisFinishInput {
        analyses,
        session_id,
        completed_at_us,
        ids,
    } = input;
    let mut commands: Option<WriteBatch> = None;
    let mut snapshots = Vec::new();
    for (&snapshot_id, active) in analyses {
        if active.session_id != session_id {
            continue;
        }
        let outcome = context_snapshot_outcome(ContextSnapshotOutcomeInput {
            snapshot_id,
            status: AnalyzerAnalysisStatus::Partial,
            completed_at_us,
            explicit_block_count: Some(u64::from(active.next_ordinal)),
        });
        let event =
            context_analysis_partial_event_with_snapshot(ContextAnalysisPartialEventInput {
                active,
                snapshot_id,
                completed_at_us,
                ids,
            });
        commands = Some(match commands {
            Some(batch) => batch.and(outcome).and(event),
            None => WriteBatch::new(outcome).and(event),
        });
        snapshots.push(snapshot_id);
    }
    (commands, snapshots)
}

fn context_analysis_partial_event_with_snapshot(
    input: ContextAnalysisPartialEventInput<'_, '_>,
) -> WriteCommand {
    let ContextAnalysisPartialEventInput {
        active,
        snapshot_id,
        completed_at_us,
        ids,
    } = input;
    let payload = format!(
        r#"{{"snapshot_id":"{snapshot_id}","status":"partial","analysis_version":{},"explicit_block_count":{}}}"#,
        active.analysis_version,
        active.next_ordinal
    )
    .into_bytes()
    .into_boxed_slice();
    WriteCommand::Event {
        event_id: EventId::generate(ids),
        session_id: Some(active.session_id),
        operation_id: Some(active.inference_operation_id),
        timestamp: completed_at_us.to_string(),
        event_type: "context.analysis.partial".to_owned(),
        payload,
        schema_version: "1".to_owned(),
    }
}

/// Sole daemon owner of lifecycle memory and durable writes.
#[derive(Debug)]
pub struct DaemonService {
    writer: StorageWriter,
    ids: Mutex<UuidV7Generator>,
    sessions: Mutex<HashMap<SessionId, SessionRecord>>,
    pub(crate) context_analyses: Mutex<HashMap<ContextSnapshotId, ActiveContextAnalysis>>,
    recovered_stale_sessions: u64,
}

impl DaemonService {
    /// Opens a daemon service and marks interrupted sessions from a prior process stale.
    ///
    /// # Errors
    /// Returns a storage error when startup recovery cannot commit.
    pub async fn open(writer: StorageWriter, recovered_at: &str) -> Result<Self, DaemonError> {
        let recovered = writer.recover_stale_sessions(recovered_at).await?;
        let recovered_stale_sessions = recovered.recovered_sessions;
        let service = Self {
            writer,
            ids: Mutex::new(UuidV7Generator::new()),
            sessions: Mutex::new(HashMap::new()),
            context_analyses: Mutex::new(HashMap::new()),
            recovered_stale_sessions,
        };
        tracing::info!(
            recovered_stale_sessions,
            "daemon startup recovery completed"
        );
        Ok(service)
    }

    /// Returns how many interrupted sessions startup recovery marked stale.
    #[must_use]
    pub const fn recovered_stale_sessions(&self) -> u64 {
        self.recovered_stale_sessions
    }

    /// Creates and durably registers an isolated active session.
    ///
    /// # Errors
    /// Returns a storage error when the session cannot be committed.
    pub async fn start_session(&self, started_at: &str) -> Result<SessionSnapshot, DaemonError> {
        let ids = self.ids.lock().await;
        let session_id = SessionId::generate(&ids);
        drop(ids);
        let ingress_key = format!("session-{session_id}");
        let _receipt = self
            .writer
            .submit(WriteCommand::Session {
                session_id,
                started_at: started_at.to_owned(),
                ended_at: None,
                state: SessionState::Active,
                ingress_key: ingress_key.clone(),
            })
            .await?;
        tracing::info!(%session_id, %ingress_key, "session started");
        let record = SessionRecord {
            state: SessionState::Active,
            ingress_key: ingress_key.clone(),
            operations: HashMap::new(),
            provider_requests: HashMap::new(),
            next_attempt_ordinals: HashMap::new(),
        };
        let previous = self.sessions.lock().await.insert(session_id, record);
        debug_assert!(previous.is_none());
        Ok(SessionSnapshot {
            session_id,
            state: SessionState::Active,
            ingress_key,
            operations: 0,
        })
    }

    /// Creates one operation and optionally commits its parent edge atomically.
    ///
    /// # Errors
    /// Returns a typed lifecycle, causal-edge, or storage error.
    #[allow(
        clippy::too_many_arguments,
        clippy::significant_drop_tightening,
        reason = "the lifecycle transaction keeps its typed request fields and session lock atomic"
    )]
    pub async fn create_operation(
        &self,
        session_id: SessionId,
        kind: OperationKind,
        started_at: &str,
        parent: Option<(OperationId, CausalRelationship)>,
    ) -> Result<OperationId, DaemonError> {
        let mut sessions = self.sessions.lock().await;
        let record = sessions
            .get_mut(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        if record.state != SessionState::Active {
            return Err(DaemonError::SessionNotActive {
                session_id,
                state: record.state,
            });
        }
        if let Some((parent_id, _relationship)) = parent {
            if !record.operations.contains_key(&parent_id) {
                return Err(DaemonError::UnknownOperation {
                    session_id,
                    operation_id: parent_id,
                });
            }
        }
        let ids = self.ids.lock().await;
        let operation_id = OperationId::generate(&ids);
        drop(ids);
        let operation = WriteCommand::Operation {
            operation_id,
            session_id,
            kind,
            started_at: started_at.to_owned(),
            ended_at: None,
            status: OperationStatus::Started,
        };
        let batch = match parent {
            Some((parent_id, relationship)) => {
                WriteBatch::new(operation).and(WriteCommand::CausalEdge {
                    edge: CausalEdge::new(parent_id, operation_id, relationship)?,
                })
            }
            None => WriteBatch::new(operation),
        };
        let _receipt = self.writer.submit_batch(batch).await?;
        tracing::info!(%session_id, %operation_id, ?kind, "operation started");
        let inserted = record.operations.insert(operation_id, kind);
        debug_assert!(inserted.is_none());
        Ok(operation_id)
    }

    /// Records one provider observation beneath an existing root operation.
    ///
    /// The inference operation and its causal edge commit in one atomic batch before any
    /// provider row is written, so a recorded observation never leaves an operation without a
    /// parent. Each forwarded provider request produces exactly one inference operation, one
    /// logical request, and one attempt.
    ///
    /// # Errors
    /// Returns [`DaemonError::UnknownSession`] or [`DaemonError::SessionNotActive`] for a session
    /// this daemon cannot accept work for, [`DaemonError::UnknownOperation`] when the root
    /// operation is not registered in that session, or a storage error when a batch cannot
    /// commit.
    pub async fn record_provider_observation(
        &self,
        input: RecordProviderObservation,
    ) -> Result<RecordedProviderObservation, DaemonError> {
        let RecordProviderObservation {
            session_id,
            parent_operation_id,
            observation,
        } = input;
        let operation_kind = match observation.request.request_kind {
            ProviderRequestKind::Compaction => OperationKind::ContextCompaction,
            _ => OperationKind::LlmInference,
        };
        let operation_id = self
            .create_operation(
                session_id,
                operation_kind,
                &observation.started_at,
                Some((parent_operation_id, CausalRelationship::Spawned)),
            )
            .await?;
        let receipt = self
            .persist_provider_observation(PersistProviderObservation::new(
                session_id,
                operation_id,
                observation,
            ))
            .await?;
        Ok(RecordedProviderObservation {
            operation_id,
            receipt,
        })
    }

    /// Persists one provider observation against an existing LLM inference.
    ///
    /// The first observation for an inference creates the logical request. Later observations
    /// reuse that request identity and create only another attempt. All rows belonging to this
    /// observation are submitted through the sole writer in one transaction.
    ///
    /// # Errors
    /// Returns a lifecycle, association, or storage error. No rows are retained when the batch
    /// fails.
    ///
    /// # Panics
    /// Panics if the storage writer violates its atomic-batch receipt contract.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the session record must remain locked until the durable batch commits so request identities and attempt ordinals stay unique"
    )]
    pub async fn persist_provider_observation(
        &self,
        input: PersistProviderObservation,
    ) -> Result<ProviderObservationReceipt, DaemonError> {
        let PersistProviderObservation {
            session_id,
            operation_id,
            observation,
        } = input;
        let mut sessions = self.sessions.lock().await;
        let record = sessions
            .get_mut(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        if record.state != SessionState::Active {
            return Err(DaemonError::SessionNotActive {
                session_id,
                state: record.state,
            });
        }
        let kind = *record
            .operations
            .get(&operation_id)
            .ok_or(DaemonError::UnknownOperation {
                session_id,
                operation_id,
            })?;
        if !matches!(
            kind,
            OperationKind::LlmInference | OperationKind::ContextCompaction
        ) {
            return Err(DaemonError::OperationNotInference {
                session_id,
                operation_id,
                kind,
            });
        }

        let request_id = if let Some(request_id) = record.provider_requests.get(&operation_id) {
            *request_id
        } else {
            let ids = self.ids.lock().await;
            let request_id = RequestId::generate(&ids);
            drop(ids);
            request_id
        };
        let ordinal = *record
            .next_attempt_ordinals
            .get(&operation_id)
            .unwrap_or(&0);
        let next =
            ordinal
                .checked_add(1)
                .ok_or(DaemonError::Storage(StorageError::IntegerOverflow {
                    field: "attempt_ordinal",
                    value: ordinal,
                }))?;
        let generator = self.ids.lock().await;
        let attempt_id = AttemptId::generate(&generator);
        let batch = provider_observation_batch(
            ProviderWriteIds {
                session_id,
                operation_id,
                request_id,
                attempt_id,
                ordinal,
                is_retry: record.provider_requests.contains_key(&operation_id),
            },
            &observation,
            &generator,
        );
        drop(generator);
        let receipt = self.writer.submit_batch(batch).await?;
        assert!(
            matches!(receipt, WriteReceipt::BatchCommitted { rows_changed } if rows_changed > 0),
            "provider observation must commit a non-empty atomic batch: {receipt:?}"
        );
        let previous_request_id = record.provider_requests.insert(operation_id, request_id);
        assert!(
            previous_request_id.is_none() || previous_request_id == Some(request_id),
            "provider request identity changed for operation {operation_id}"
        );
        let previous_ordinal = record.next_attempt_ordinals.insert(operation_id, next);
        assert_eq!(
            previous_ordinal,
            (ordinal != 0).then_some(ordinal),
            "provider attempt ordinal state changed unexpectedly for operation {operation_id}"
        );
        Ok(ProviderObservationReceipt {
            request_id,
            attempt_id,
            ordinal,
        })
    }

    /// Transitions an active session to a terminal state after durable commit.
    ///
    /// # Errors
    /// Returns a typed error for an unknown/non-active session or failed durable update.
    #[allow(
        clippy::too_many_arguments,
        clippy::significant_drop_tightening,
        reason = "the terminal transition keeps identity, state, timestamp, and lock atomic"
    )]
    pub async fn finish_session(
        &self,
        session_id: SessionId,
        state: SessionState,
        ended_at: &str,
    ) -> Result<SessionSnapshot, DaemonError> {
        let mut sessions = self.sessions.lock().await;
        {
            let record = sessions
                .get(&session_id)
                .ok_or(DaemonError::UnknownSession { session_id })?;
            if record.state != SessionState::Active && record.state != SessionState::Closing {
                return Err(DaemonError::SessionNotActive {
                    session_id,
                    state: record.state,
                });
            }
        }

        let mut analyses = self.context_analyses.lock().await;
        let ids = self.ids.lock().await;
        let minimum_completed_at_us = analyses
            .values()
            .filter(|active| active.session_id == session_id)
            .map(|active| active.started_at_us)
            .max()
            .unwrap_or(0);
        let completed_at_us = ended_at
            .parse::<u64>()
            .unwrap_or(minimum_completed_at_us)
            .max(minimum_completed_at_us);
        let (context_batch, snapshots_to_remove) =
            context_analysis_finish_batch(ContextAnalysisFinishInput {
                analyses: &analyses,
                session_id,
                completed_at_us,
                ids: &ids,
            });
        let session_command = WriteCommand::SessionState {
            session_id,
            ended_at: Some(ended_at.to_owned()),
            state,
        };
        let write_batch = match context_batch {
            Some(batch) => batch.and(session_command),
            None => WriteBatch::new(session_command),
        };
        drop(ids);
        let _receipt = self.writer.submit_batch(write_batch).await?;
        for snapshot_id in snapshots_to_remove {
            let _removed = analyses.remove(&snapshot_id);
        }
        let record = sessions
            .get_mut(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        record.state = state;
        tracing::info!(%session_id, ?state, "session finished");
        let result = snapshot(session_id, record);
        drop(analyses);
        drop(sessions);
        Ok(result)
    }

    /// Records one correlation degradation that left no forward record to carry it.
    ///
    /// A degradation the recorder could attach to a forward travels with that forward's
    /// observation and commits inside its atomic batch. This path exists for the degradations
    /// that have no record at all — a half refused because its identity was already retired, or
    /// terminal evidence whose request half never arrived — so the loss is still observable.
    ///
    /// # Errors
    /// Returns [`DaemonError::UnknownSession`] or [`DaemonError::SessionNotActive`] for a session
    /// this daemon cannot accept work for, or a storage error when the event cannot commit.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the session guard must outlive the durable append so no event lands on a session this daemon has already closed"
    )]
    pub async fn record_correlation_degradation(
        &self,
        input: RecordCorrelationDegradation,
    ) -> Result<(), DaemonError> {
        let RecordCorrelationDegradation {
            session_id,
            reason,
            observed_at,
        } = input;
        let sessions = self.sessions.lock().await;
        let record = sessions
            .get(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        if record.state != SessionState::Active {
            return Err(DaemonError::SessionNotActive {
                session_id,
                state: record.state,
            });
        }
        let generator = self.ids.lock().await;
        let event = correlation_degraded_event(
            CorrelationDegraded {
                session_id,
                operation_id: None,
                reason,
                timestamp: &observed_at,
            },
            &generator,
        );
        drop(generator);
        let _receipt = self.writer.submit(event).await?;
        tracing::info!(%session_id, ?reason, "correlation degraded");
        Ok(())
    }

    /// Records one aggregate event for analyses dropped before context persistence.
    ///
    /// The event carries only the session, reason, count, and timestamp. It never carries
    /// request content, block data, or provider usage.
    ///
    /// # Errors
    /// Returns [`DaemonError::UnknownSession`] or [`DaemonError::SessionNotActive`] when the
    /// session cannot accept another event, or a storage error when the event cannot commit.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the session guard must outlive the durable append"
    )]
    pub async fn record_context_analysis_dropped(
        &self,
        input: RecordContextAnalysisDropped,
    ) -> Result<(), DaemonError> {
        let RecordContextAnalysisDropped {
            session_id,
            reason,
            dropped,
            observed_at_us,
        } = input;
        let sessions = self.sessions.lock().await;
        let record = sessions
            .get(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        if record.state != SessionState::Active {
            return Err(DaemonError::SessionNotActive {
                session_id,
                state: record.state,
            });
        }
        let generator = self.ids.lock().await;
        let payload = context_analysis_drop_payload(session_id, reason, dropped);
        let event = WriteCommand::Event {
            event_id: EventId::generate(&generator),
            session_id: Some(session_id),
            operation_id: None,
            timestamp: observed_at_us.to_string(),
            event_type: "context.analysis.dropped".to_owned(),
            payload,
            schema_version: CONTEXT_EVENT_SCHEMA_VERSION.to_owned(),
        };
        drop(generator);
        let _receipt = self.writer.submit(event).await?;
        tracing::info!(%session_id, ?reason, dropped, "context analyses dropped");
        Ok(())
    }

    /// Returns one bounded metadata-only context inspection through daemon-owned storage.
    ///
    /// # Errors
    /// Returns [`DaemonError::ContextInspectionNotFound`] when no finalized snapshot exists for
    /// the request, or a storage error for the read boundary.
    pub async fn inspect_context(
        &self,
        request_id: RequestId,
    ) -> Result<ContextInspection, DaemonError> {
        match self.writer.inspect_context(request_id).await {
            Ok(inspection) => Ok(inspection),
            Err(StorageError::ContextSnapshotNotFound { request_id }) => {
                Err(DaemonError::ContextInspectionNotFound { request_id })
            }
            Err(error) => Err(DaemonError::Storage(error)),
        }
    }

    /// Returns one compact durable-or-active context snapshot lifecycle status.
    ///
    /// Durable storage wins when the snapshot has committed a terminal outcome. An active
    /// snapshot is returned as nonterminal only when its begin row is not visible yet.
    ///
    /// # Errors
    /// Returns a storage error for the read boundary.
    pub async fn context_snapshot_status(
        &self,
        request_id: Option<RequestId>,
        snapshot_id: Option<ContextSnapshotId>,
    ) -> Result<Option<ContextSnapshotStatus>, DaemonError> {
        let durable = match snapshot_id {
            Some(snapshot_id) => {
                self.writer
                    .context_snapshot_status_by_snapshot(snapshot_id)
                    .await?
            }
            None => match request_id {
                Some(request_id) => {
                    self.writer
                        .context_snapshot_status_by_request(request_id)
                        .await?
                }
                None => None,
            },
        };
        if durable.is_some() {
            return Ok(durable);
        }

        let analyses = self.context_analyses.lock().await;
        let active = snapshot_id
            .and_then(|id| analyses.get(&id).map(|analysis| (id, analysis)))
            .or_else(|| {
                request_id.and_then(|request| {
                    analyses
                        .iter()
                        .find(|(_, analysis)| analysis.provider_request_id == request)
                        .map(|(id, analysis)| (*id, analysis))
                })
            });
        Ok(active.map(|(snapshot_id, analysis)| {
            ContextSnapshotStatus::new(
                snapshot_id,
                analysis.provider_request_id,
                "partial".to_owned(),
            )
        }))
    }

    /// Returns the current in-memory state for one session.
    ///
    /// # Errors
    /// Returns an error when the session is unknown.
    pub async fn session(&self, session_id: SessionId) -> Result<SessionSnapshot, DaemonError> {
        self.sessions
            .lock()
            .await
            .get(&session_id)
            .map(|record| snapshot(session_id, record))
            .ok_or(DaemonError::UnknownSession { session_id })
    }

    /// Drains durable writes and stops the service.
    ///
    /// # Errors
    /// Returns a typed storage error if the writer cannot drain or join.
    pub async fn shutdown(self) -> Result<(), DaemonError> {
        self.writer.shutdown().await.map_err(Into::into)
    }
}
const fn storage_provider(value: CanonicalProviderKind) -> Option<ProviderKind> {
    match value {
        CanonicalProviderKind::OpenAi => Some(ProviderKind::OpenAi),
        _ => None,
    }
}

const fn storage_protocol(value: CanonicalProviderProtocol) -> Option<ProviderProtocol> {
    match value {
        CanonicalProviderProtocol::OpenAiResponsesV1 => Some(ProviderProtocol::OpenAiResponsesV1),
        _ => None,
    }
}

const fn storage_transport(value: CanonicalProviderTransport) -> Option<ProviderTransport> {
    match value {
        CanonicalProviderTransport::OpenAiPublicApi => Some(ProviderTransport::OpenAiPublicApi),
        CanonicalProviderTransport::ChatGptCodexSubscription => {
            Some(ProviderTransport::ChatGptCodexSubscription)
        }
        _ => None,
    }
}

const fn storage_content_encoding(value: ProviderContentEncoding) -> Option<ContentEncoding> {
    match value {
        ProviderContentEncoding::Identity => Some(ContentEncoding::Identity),
        ProviderContentEncoding::Zstd => Some(ContentEncoding::Zstd),
        ProviderContentEncoding::Unsupported => Some(ContentEncoding::Unsupported),
        _ => None,
    }
}

const fn storage_decode_status(
    value: ProviderAnalysisDecodeStatus,
) -> Option<AnalysisDecodeStatus> {
    match value {
        ProviderAnalysisDecodeStatus::Identity => Some(AnalysisDecodeStatus::Identity),
        ProviderAnalysisDecodeStatus::Decoded => Some(AnalysisDecodeStatus::Decoded),
        ProviderAnalysisDecodeStatus::UnsupportedEncoding => {
            Some(AnalysisDecodeStatus::UnsupportedEncoding)
        }
        ProviderAnalysisDecodeStatus::CorruptPayload => Some(AnalysisDecodeStatus::CorruptPayload),
        ProviderAnalysisDecodeStatus::ResourceLimit => Some(AnalysisDecodeStatus::ResourceLimit),
        ProviderAnalysisDecodeStatus::Timeout => Some(AnalysisDecodeStatus::Timeout),
        _ => None,
    }
}

const fn storage_observation_status(value: ProviderObservationStatus) -> Option<ObservationStatus> {
    match value {
        ProviderObservationStatus::Complete => Some(ObservationStatus::Complete),
        ProviderObservationStatus::Partial => Some(ObservationStatus::Partial),
        ProviderObservationStatus::Unsupported => Some(ObservationStatus::Unsupported),
        ProviderObservationStatus::Malformed => Some(ObservationStatus::Malformed),
        ProviderObservationStatus::ResourceLimit => Some(ObservationStatus::ResourceLimit),
        ProviderObservationStatus::ObserverBackpressure => {
            Some(ObservationStatus::ObserverBackpressure)
        }
        ProviderObservationStatus::Cancelled => Some(ObservationStatus::Cancelled),
        _ => None,
    }
}

const fn storage_response_state(value: ProviderResponseState) -> Option<StorageResponseState> {
    match value {
        ProviderResponseState::Queued => Some(StorageResponseState::Queued),
        ProviderResponseState::InProgress => Some(StorageResponseState::InProgress),
        ProviderResponseState::Completed => Some(StorageResponseState::Completed),
        ProviderResponseState::Incomplete => Some(StorageResponseState::Incomplete),
        ProviderResponseState::Failed => Some(StorageResponseState::Failed),
        ProviderResponseState::Cancelled => Some(StorageResponseState::Cancelled),
        ProviderResponseState::Disconnected => Some(StorageResponseState::Disconnected),
        ProviderResponseState::Unknown => Some(StorageResponseState::Unknown),
        _ => None,
    }
}

const fn storage_usage_status(value: ProviderUsageStatus) -> Option<UsageStatus> {
    match value {
        ProviderUsageStatus::Final => Some(UsageStatus::Final),
        ProviderUsageStatus::Partial => Some(UsageStatus::Partial),
        ProviderUsageStatus::Unavailable => Some(UsageStatus::Unavailable),
        _ => None,
    }
}

/// Canonical event names committed beside the relational rows of one observation.
const EVENT_REQUEST_OBSERVED: &str = "provider.request.observed";
const EVENT_RESPONSE_STARTED: &str = "provider.response.started";
const EVENT_RESPONSE_COMPLETED: &str = "provider.response.completed";
const EVENT_RESPONSE_INCOMPLETE: &str = "provider.response.incomplete";
const EVENT_RESPONSE_FAILED: &str = "provider.response.failed";
const EVENT_USAGE_OBSERVED: &str = "provider.usage.observed";
const EVENT_USAGE_NORMALIZED: &str = "provider.usage.normalized";
const EVENT_OBSERVATION_PARTIAL: &str = "provider.observation.partial";

/// Payload schema version shared by every canonical provider observation event.
const PROVIDER_EVENT_SCHEMA_VERSION: &str = "1";

/// Canonical event name of one observed correlation degradation.
const EVENT_CORRELATION_DEGRADED: &str = "context.correlation.degraded";

/// Payload schema version shared by every canonical context event.
const CONTEXT_EVENT_SCHEMA_VERSION: &str = "1";

/// Identities and reason of one correlation degradation about to be committed.
#[derive(Clone, Copy)]
struct CorrelationDegraded<'evidence> {
    session_id: SessionId,
    /// Inference operation of the degraded forward, when the degradation left one behind.
    operation_id: Option<OperationId>,
    reason: CorrelationDegradation,
    timestamp: &'evidence str,
}

/// Builds the canonical event of one correlation degradation.
///
/// The payload carries the reason and the session only. A degradation is the absence of
/// evidence, so nothing about the forward itself — no request, no status, no counts, no
/// content — belongs in it; the row's own identities keep it joinable to the session and, when
/// the degraded forward was recorded, to its inference operation.
fn correlation_degraded_event(
    degradation: CorrelationDegraded<'_>,
    generator: &UuidV7Generator,
) -> WriteCommand {
    let payload = serde_json::json!({
        "reason": degradation.reason,
        "session_id": degradation.session_id.to_string(),
    });
    WriteCommand::Event {
        event_id: EventId::generate(generator),
        session_id: Some(degradation.session_id),
        operation_id: degradation.operation_id,
        timestamp: degradation.timestamp.to_owned(),
        event_type: EVENT_CORRELATION_DEGRADED.to_owned(),
        // `Value` renders its own bytes, so an event never depends on a fallible re-encode.
        payload: payload.to_string().into_bytes().into(),
        schema_version: CONTEXT_EVENT_SCHEMA_VERSION.to_owned(),
    }
}

/// Whether an observed upstream status reports a failed exchange.
fn errored_upstream_status(status_code: Option<HttpStatusCode>) -> bool {
    status_code.is_some_and(|code| !(200..300).contains(&code.get()))
}

/// Derives the attempt lifecycle from every piece of evidence the observation carries.
///
/// Transport failure, a non-2xx upstream status, and a provider error code are terminal error
/// evidence whatever the semantic state says, so a failed attempt is never indistinguishable
/// from one still in flight. Cancellation, disconnection, and incompleteness stay distinct, and
/// `Completed` is reported only when the provider itself reported a completed response with no
/// error evidence beside it.
fn inference_status(observation: &ProviderObservation) -> InferenceStatus {
    if matches!(
        observation.request.request_kind,
        ProviderRequestKind::Compaction
    ) {
        if observation.transport_error.is_some()
            || errored_upstream_status(observation.status_code)
            || observation.outcome == ProviderObservationOutcome::Failed
        {
            return InferenceStatus::Errored;
        }
        return match observation.outcome {
            ProviderObservationOutcome::Completed => InferenceStatus::Completed,
            ProviderObservationOutcome::Incomplete => InferenceStatus::Incomplete,
            ProviderObservationOutcome::Cancelled => InferenceStatus::Cancelled,
            ProviderObservationOutcome::Disconnected => InferenceStatus::Disconnected,
            ProviderObservationOutcome::Failed => InferenceStatus::Errored,
            ProviderObservationOutcome::InProgress => {
                if observation.ended_at.is_some() {
                    InferenceStatus::Incomplete
                } else {
                    InferenceStatus::Started
                }
            }
        };
    }
    if observation.transport_error.is_some() {
        return InferenceStatus::Errored;
    }
    let errored = errored_upstream_status(observation.status_code)
        || observation.outcome == ProviderObservationOutcome::Failed;
    let Some(response) = observation.response.as_ref() else {
        if errored {
            return InferenceStatus::Errored;
        }
        return match observation.outcome {
            ProviderObservationOutcome::Cancelled => InferenceStatus::Cancelled,
            ProviderObservationOutcome::Disconnected => InferenceStatus::Disconnected,
            ProviderObservationOutcome::Failed => InferenceStatus::Errored,
            ProviderObservationOutcome::Incomplete | ProviderObservationOutcome::Completed => {
                InferenceStatus::Incomplete
            }
            // Only a forward with no terminal evidence at all is still running.
            ProviderObservationOutcome::InProgress => {
                if observation.ended_at.is_some() {
                    InferenceStatus::Incomplete
                } else {
                    InferenceStatus::Started
                }
            }
        };
    };
    if response.status == ProviderObservationStatus::Cancelled
        || response.response_state == ProviderResponseState::Cancelled
        || observation.outcome == ProviderObservationOutcome::Cancelled
    {
        return InferenceStatus::Cancelled;
    }
    if errored
        || response.error_code.is_some()
        || response.response_state == ProviderResponseState::Failed
    {
        return InferenceStatus::Errored;
    }
    match response.response_state {
        ProviderResponseState::Completed => InferenceStatus::Completed,
        ProviderResponseState::Disconnected => InferenceStatus::Disconnected,
        // The observation reached its terminal decision, so a response the provider never
        // reported terminal ended without complete output instead of still running.
        _ => match observation.outcome {
            ProviderObservationOutcome::Disconnected => InferenceStatus::Disconnected,
            _ => InferenceStatus::Incomplete,
        },
    }
}

/// Terminal operation state implied by an attempt lifecycle, if the attempt finished.
const fn terminal_operation_status(status: InferenceStatus) -> Option<OperationStatus> {
    match status {
        InferenceStatus::Completed => Some(OperationStatus::Completed),
        InferenceStatus::Incomplete => Some(OperationStatus::Incomplete),
        InferenceStatus::Cancelled => Some(OperationStatus::Cancelled),
        InferenceStatus::Errored => Some(OperationStatus::Errored),
        InferenceStatus::Disconnected => Some(OperationStatus::Disconnected),
        InferenceStatus::Started | InferenceStatus::Streaming => None,
    }
}

/// Whether the observed response was streamed, from the request hint or observed chunks.
fn streaming_flag(observation: &ProviderObservation) -> Option<bool> {
    observation.streaming.or_else(|| {
        observation
            .response
            .as_ref()
            .and_then(|response| response.chunk_count.map(|_chunks| true))
    })
}

fn anomaly_metadata(flags: AnomalyFlags) -> Option<String> {
    if !flags.any() {
        return None;
    }
    serde_json::to_string(&serde_json::json!({
        "invalid_number": flags.invalid_number,
        "overflow": flags.overflow,
        "cached_exceeds_input": flags.cached_exceeds_input,
        "reasoning_exceeds_output": flags.reasoning_exceeds_output,
        "inconsistent_total": flags.inconsistent_total,
    }))
    .ok()
}

#[derive(Clone, Copy)]
struct ProviderWriteIds {
    session_id: SessionId,
    operation_id: OperationId,
    request_id: RequestId,
    attempt_id: AttemptId,
    ordinal: u64,
    is_retry: bool,
}

/// One observation with the identities and lifecycle the batch derived for it.
#[derive(Clone, Copy)]
struct ObservedAttempt<'evidence> {
    ids: ProviderWriteIds,
    observation: &'evidence ProviderObservation,
    status: InferenceStatus,
}

/// Builds the one atomic transaction that records everything one observation established.
///
/// The relational rows stay the query model; the canonical events supplement them, and the
/// terminal operation state closes the inference so the DAG never shows a finished forward as
/// permanently in flight. All of it commits together or not at all.
fn provider_observation_batch(
    ids: ProviderWriteIds,
    observation: &ProviderObservation,
    generator: &UuidV7Generator,
) -> WriteBatch {
    let evidence = ObservedAttempt {
        ids,
        observation,
        status: inference_status(observation),
    };
    let attempt = provider_attempt_command(evidence);
    let mut batch = if ids.is_retry {
        WriteBatch::new(attempt)
    } else {
        WriteBatch::new(provider_request_command(
            ids.operation_id,
            ids.request_id,
            observation,
        ))
        .and(attempt)
    };
    if let Some(response) = observation.response.as_ref() {
        batch = batch.and(provider_usage_command(ids.attempt_id, response));
    }
    for event in provider_events(evidence, generator) {
        batch = batch.and(event);
    }
    // A forward the recorder could not fully correlate keeps every row it has evidence for, and
    // says so in the same transaction: a record whose degradation was committed separately
    // would be readable for a moment as complete causality.
    if let CorrelationStatus::Degraded(reason) = observation.correlation_status {
        batch = batch.and(correlation_degraded_event(
            CorrelationDegraded {
                session_id: ids.session_id,
                operation_id: Some(ids.operation_id),
                reason,
                timestamp: observation
                    .ended_at
                    .as_deref()
                    .unwrap_or(observation.started_at.as_str()),
            },
            generator,
        ));
    }
    if let Some(operation_status) = terminal_operation_status(evidence.status) {
        batch = batch.and(WriteCommand::OperationState {
            operation_id: ids.operation_id,
            ended_at: observation.ended_at.clone(),
            status: operation_status,
        });
    }
    batch
}

fn provider_request_command(
    operation_id: OperationId,
    request_id: RequestId,
    observation: &ProviderObservation,
) -> WriteCommand {
    let request = &observation.request;
    let compaction = matches!(request.request_kind, ProviderRequestKind::Compaction);
    let metadata = if matches!(request.request_kind, ProviderRequestKind::Compaction) {
        RequestMetadata::responses_compact(request_id, observation.request_bytes)
    } else {
        RequestMetadata::responses(request_id, observation.request_bytes)
    };
    WriteCommand::ProviderRequest {
        operation_id,
        metadata,
        provider: storage_provider(request.provider),
        protocol: storage_protocol(request.protocol),
        transport: storage_transport(request.transport),
        endpoint_profile_version: request.endpoint_profile_version,
        content_encoding: storage_content_encoding(request.content_encoding),
        analysis_decode_status: (!compaction)
            .then(|| storage_decode_status(request.analysis_decode_status))
            .flatten(),
        wire_bytes: request.wire_bytes,
        wire_sha256: request.wire_sha256.clone(),
        decoded_bytes: (!compaction).then_some(request.decoded_bytes).flatten(),
        decode_duration_us: (!compaction)
            .then_some(request.decode_duration_us)
            .flatten(),
        decoder_version: (!compaction).then_some(request.decoder_version).flatten(),
        parser_version: (!compaction).then_some(request.parser_version),
        observation_status: storage_observation_status(request.status),
        model: request.model.clone(),
        stream: request.stream,
        background: request.background,
        store: request.store,
        reasoning_effort: request.reasoning_effort.clone(),
        text_verbosity: request.verbosity.clone(),
        truncation: request.truncation.clone(),
        // Compaction is intentionally transport-only: its semantic request fields were never
        // parsed, so false would incorrectly turn "unknown" into a factual absence.
        previous_response_id_present: (!compaction).then_some(request.has_previous_response_id),
        input_item_count: request.input_item_count,
        tool_count: request.tool_count,
        text_input_block_count: request.text_input_blocks,
        image_input_block_count: request.image_input_blocks,
        file_input_block_count: request.file_input_blocks,
    }
}

fn provider_attempt_command(evidence: ObservedAttempt<'_>) -> WriteCommand {
    let ObservedAttempt {
        ids,
        observation,
        status,
    } = evidence;
    let response = observation.response.as_ref();
    WriteCommand::ProviderAttempt {
        attempt_id: ids.attempt_id,
        request_id: ids.request_id,
        ordinal: ids.ordinal,
        status_code: observation.status_code,
        started_at: observation.started_at.clone(),
        ended_at: observation.ended_at.clone(),
        status,
        provider_response_id: response.and_then(|value| value.provider_response_id.clone()),
        response_model: response.and_then(|value| value.model.clone()),
        response_state: response.and_then(|value| storage_response_state(value.response_state)),
        provider_created_at: response
            .and_then(|value| value.created_at.map(|timestamp| timestamp.to_string())),
        incomplete_reason: response.and_then(|value| value.incomplete_reason.clone()),
        error_code: response.and_then(|value| value.error_code.clone()),
        transport_error: observation.transport_error.clone(),
        observation_status: response
            .and_then(|value| storage_observation_status(value.status))
            .or_else(|| storage_observation_status(observation.request.status)),
        streaming: streaming_flag(observation),
        chunk_count: response.and_then(|value| value.chunk_count),
        byte_count: response
            .and_then(|value| value.byte_count)
            .or(observation.response_bytes),
        // Only measurements the observer actually took are persisted; unknown stays NULL.
        ttfb_us: response.and_then(|value| value.ttfb_us),
        ttft_us: response.and_then(|value| value.ttft_us),
        duration_us: response
            .and_then(|value| value.duration_us)
            .or(observation.duration_us),
        anomaly_metadata: response.and_then(|value| {
            value
                .normalized_usage
                .as_ref()
                .and_then(|usage| anomaly_metadata(usage.anomalies))
        }),
    }
}

fn provider_usage_command(attempt_id: AttemptId, response: &ResponseObservation) -> WriteCommand {
    let normalized = response.normalized_usage.as_ref();
    WriteCommand::ProviderUsage {
        attempt_id,
        input_total: normalized.and_then(|usage| usage.input_total),
        input_uncached: normalized.and_then(|usage| usage.input_uncached),
        cache_read: normalized.and_then(|usage| usage.input_cached),
        cache_write: normalized.and_then(|usage| usage.cache_write),
        output_total: normalized.and_then(|usage| usage.output_total),
        reasoning: normalized.and_then(|usage| usage.output_reasoning),
        usage_status: storage_usage_status(response.usage_status),
        raw_usage_json: response.raw_usage.as_ref().map(|raw| raw.as_bytes().into()),
        input_cached: normalized.and_then(|usage| usage.input_cached),
        output_reasoning: normalized.and_then(|usage| usage.output_reasoning),
        total: normalized.and_then(|usage| usage.total),
        normalizer_version: normalized.map(|usage| usage.normalizer_version),
        anomaly_metadata: normalized.and_then(|usage| anomaly_metadata(usage.anomalies)),
    }
}

/// Accumulates the canonical events of one observation with their shared identities.
struct ProviderEvents<'evidence> {
    ids: ProviderWriteIds,
    timestamp: &'evidence str,
    generator: &'evidence UuidV7Generator,
    commands: Vec<WriteCommand>,
}

impl ProviderEvents<'_> {
    /// Appends one canonical event whose payload carries only allowlisted metadata.
    fn push(&mut self, event_type: &str, fields: serde_json::Value) {
        let mut payload = serde_json::Map::from_iter([
            (
                "operation_id".to_owned(),
                serde_json::Value::String(self.ids.operation_id.to_string()),
            ),
            (
                "request_id".to_owned(),
                serde_json::Value::String(self.ids.request_id.to_string()),
            ),
            (
                "attempt_id".to_owned(),
                serde_json::Value::String(self.ids.attempt_id.to_string()),
            ),
            (
                "attempt_ordinal".to_owned(),
                serde_json::Value::from(self.ids.ordinal),
            ),
        ]);
        if let serde_json::Value::Object(fields) = fields {
            payload.extend(fields);
        }
        let Ok(bytes) = serde_json::to_vec(&serde_json::Value::Object(payload)) else {
            return;
        };
        self.commands.push(WriteCommand::Event {
            event_id: EventId::generate(self.generator),
            session_id: Some(self.ids.session_id),
            operation_id: Some(self.ids.operation_id),
            timestamp: self.timestamp.to_owned(),
            event_type: event_type.to_owned(),
            payload: bytes.into(),
            schema_version: PROVIDER_EVENT_SCHEMA_VERSION.to_owned(),
        });
    }
}

/// Emits exactly the canonical events the observed evidence supports.
///
/// Payloads carry identifiers, versions, states, counts, and timings only: no headers, no
/// content, no raw usage bytes, and no sensitive provider metadata.
#[allow(
    clippy::too_many_lines,
    reason = "the canonical provider event payload keeps all allowlisted request metadata together"
)]
fn provider_events(
    evidence: ObservedAttempt<'_>,
    generator: &UuidV7Generator,
) -> Vec<WriteCommand> {
    let ObservedAttempt {
        ids,
        observation,
        status,
    } = evidence;
    let response = observation.response.as_ref();
    let mut events = ProviderEvents {
        ids,
        timestamp: observation
            .ended_at
            .as_deref()
            .unwrap_or(observation.started_at.as_str()),
        generator,
        commands: Vec::new(),
    };
    let request = &observation.request;
    let compaction = matches!(request.request_kind, ProviderRequestKind::Compaction);
    events.push(
        EVENT_REQUEST_OBSERVED,
        serde_json::json!({
            "provider": request.provider,
            "protocol": request.protocol,
            "transport": request.transport,
            "request_kind": request.request_kind,
            "endpoint_profile_version": request.endpoint_profile_version,
            "parser_version": (!compaction).then_some(request.parser_version),
            "observation_status": request.status,
            "content_encoding": request.content_encoding,
            "analysis_decode_status": (!compaction).then_some(request.analysis_decode_status),
            "wire_bytes": request.wire_bytes,
            "decoded_bytes": (!compaction).then_some(request.decoded_bytes).flatten(),
            "decode_duration_us": (!compaction).then_some(request.decode_duration_us).flatten(),
            "decoder_version": (!compaction).then_some(request.decoder_version).flatten(),
            "model": request.model,
            "stream": request.stream,
            "request_bytes": observation.request_bytes,
            "input_item_count": request.input_item_count,
            "tool_count": request.tool_count,
        }),
    );
    // The upstream response began only if its status or its bytes were observed.
    if observation.status_code.is_some() || response.is_some() {
        events.push(
            EVENT_RESPONSE_STARTED,
            serde_json::json!({
                "status_code": observation.status_code.map(HttpStatusCode::get),
                "streaming": streaming_flag(observation),
                "provider_response_id": response.and_then(|value| value.provider_response_id.as_deref()),
                "response_state": response.map(|value| value.response_state),
                "ttfb_us": response.and_then(|value| value.ttfb_us),
            }),
        );
    }
    if let Some(terminal) = terminal_event(status) {
        events.push(
            terminal,
            serde_json::json!({
                "status": status,
                "response_state": response.map(|value| value.response_state),
                "incomplete_reason": response.and_then(|value| value.incomplete_reason.as_deref()),
                "error_code": response.and_then(|value| value.error_code.as_deref()),
                "transport_error": observation.transport_error.as_deref(),
                "chunk_count": response.and_then(|value| value.chunk_count),
                "byte_count": response.and_then(|value| value.byte_count),
                "ttfb_us": response.and_then(|value| value.ttfb_us),
                "ttft_us": response.and_then(|value| value.ttft_us),
                "duration_us": response.and_then(|value| value.duration_us),
            }),
        );
    }
    if let Some(response) = response {
        if let Some(raw) = response.raw_usage.as_ref() {
            events.push(
                EVENT_USAGE_OBSERVED,
                serde_json::json!({
                    "usage_status": response.usage_status,
                    // The usage bytes themselves stay in the relational row, never in an event.
                    "raw_usage_bytes": raw.as_bytes().len(),
                }),
            );
        }
        if let Some(usage) = response.normalized_usage.as_ref() {
            events.push(
                EVENT_USAGE_NORMALIZED,
                serde_json::json!({
                    "normalizer_version": usage.normalizer_version,
                    "usage_status": usage.status,
                    "input_total": usage.input_total,
                    "input_cached": usage.input_cached,
                    "input_uncached": usage.input_uncached,
                    "cache_write": usage.cache_write,
                    "output_total": usage.output_total,
                    "output_reasoning": usage.output_reasoning,
                    "total": usage.total,
                    "anomalies": usage.anomalies,
                }),
            );
        }
    }
    let response_status = response.map(|value| value.status);
    if request.status != ProviderObservationStatus::Complete
        || response_status.is_some_and(|value| value != ProviderObservationStatus::Complete)
    {
        events.push(
            EVENT_OBSERVATION_PARTIAL,
            serde_json::json!({
                "request_observation_status": request.status,
                "response_observation_status": response_status,
            }),
        );
    }
    events.commands
}

/// The single terminal response event a finished attempt supports, if it finished.
const fn terminal_event(status: InferenceStatus) -> Option<&'static str> {
    match status {
        InferenceStatus::Completed => Some(EVENT_RESPONSE_COMPLETED),
        // The canonical vocabulary reports every non-completed terminal response as incomplete;
        // the exact state stays in the relational row.
        InferenceStatus::Incomplete
        | InferenceStatus::Cancelled
        | InferenceStatus::Disconnected => Some(EVENT_RESPONSE_INCOMPLETE),
        InferenceStatus::Errored => Some(EVENT_RESPONSE_FAILED),
        InferenceStatus::Started | InferenceStatus::Streaming => None,
    }
}

fn snapshot(session_id: SessionId, record: &SessionRecord) -> SessionSnapshot {
    SessionSnapshot {
        session_id,
        state: record.state,
        ingress_key: record.ingress_key.clone(),
        operations: record.operations.len(),
    }
}

/// Minimal authenticated control protocol used by the CLI.
#[allow(
    clippy::exhaustive_enums,
    missing_docs,
    reason = "the private CLI/daemon wire protocol has an explicit versioned boundary"
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub enum ControlRequest {
    /// Returns daemon liveness and recovery information.
    Status,
    /// Requests an orderly daemon shutdown.
    Shutdown,
    /// Starts a session owned by the caller.
    StartSession { started_at: String },
    /// Records one provider forwarding operation beneath the agent root.
    RecordForward {
        session_id: SessionId,
        parent_operation_id: OperationId,
        observed_at: String,
    },
    /// Records one semantic provider observation beneath the agent root.
    RecordProviderObservation {
        session_id: SessionId,
        parent_operation_id: OperationId,
        observation: Box<ProviderObservation>,
    },
    /// Records one correlation degradation that no forward record carries.
    ///
    /// Reason, session, and timestamp are the whole message: it is deliberately too small to
    /// carry any forward evidence, so it always fits one bounded control frame.
    RecordCorrelationDegradation {
        session_id: SessionId,
        reason: CorrelationDegradation,
        observed_at: String,
    },
    /// Records context analyses rejected before context persistence.
    RecordContextAnalysisDropped {
        session_id: SessionId,
        reason: ContextAnalysisDropReason,
        dropped: u64,
        observed_at_us: u64,
    },
    /// Atomically aborts one active context analysis and records its typed drop reason.
    AbortContextAnalysis {
        snapshot_id: ContextSnapshotId,
        reason: ContextAnalysisDropReason,
        completed_at_us: u64,
    },
    /// Finishes a previously started session.
    FinishSession {
        session_id: SessionId,
        ended_at: String,
    },
    /// Reads one bounded metadata-only context inspection by provider request identity.
    Context { request_id: RequestId },
    /// Reads one compact context snapshot lifecycle status by request or snapshot identity.
    ContextStatus {
        request_id: Option<RequestId>,
        snapshot_id: Option<ContextSnapshotId>,
    },
    /// Begins a bounded shadow context analysis for an already recorded provider request.
    BeginContextAnalysis {
        session_id: SessionId,
        provider_request_id: RequestId,
        inference_operation_id: OperationId,
        analysis_version: u32,
        started_at_us: u64,
    },
    /// Appends one ordered batch of compact context block drafts.
    AppendContextBlocks {
        snapshot_id: ContextSnapshotId,
        sequence: u32,
        blocks: Vec<tracepress_context::ContextBlockDraft>,
    },
    /// Atomically finalizes one context analysis and its derived records.
    FinalizeContextAnalysis {
        summary: Box<context::ContextAnalysisFinalize>,
    },
}

/// Response returned by the daemon control protocol.
#[allow(
    clippy::exhaustive_enums,
    missing_docs,
    reason = "the private CLI/daemon wire protocol has an explicit versioned boundary"
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlResponse {
    /// Successful response with optional lifecycle data.
    Ok {
        /// Human-readable daemon state.
        state: String,
        /// Session snapshot when a session was started.
        session: Option<SessionSnapshot>,
        /// Operation created by the request, when applicable.
        operation_id: Option<OperationId>,
        /// Provider request identity returned by provider observation recording.
        provider_request_id: Option<RequestId>,
        /// Provider attempt identity returned by provider observation recording.
        attempt_id: Option<AttemptId>,
        /// Inference operation identity returned by provider observation recording.
        inference_operation_id: Option<OperationId>,
        /// Context snapshot identity returned by `BeginContextAnalysis`.
        context_snapshot_id: Option<ContextSnapshotId>,
        /// Cumulative receipt returned by `AppendContextBlocks`.
        context_append_receipt: Option<ContextAppendReceipt>,
    },
    /// Bounded metadata-only context inspection.
    Context {
        /// Bounded metadata-only context inspection payload.
        inspection: Box<ContextInspection>,
    },
    /// Request failed inside the daemon.
    Error { message: String },
    ContextStatus {
        /// Durable or active lifecycle payload, when the snapshot is known.
        snapshot_status: Option<ContextSnapshotStatus>,
    },
}

impl ControlResponse {
    /// Creates a successful status response.
    #[must_use]
    pub fn ok(state: impl Into<String>) -> Self {
        Self::Ok {
            state: state.into(),
            session: None,
            operation_id: None,
            provider_request_id: None,
            attempt_id: None,
            inference_operation_id: None,
            context_snapshot_id: None,
            context_append_receipt: None,
        }
    }
}
