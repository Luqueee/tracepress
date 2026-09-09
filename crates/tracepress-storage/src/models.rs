use tracepress_core::{
    AttemptId, CausalEdge, ContentId, ContentObject, ContentOccurrence, ContextBinding, DecisionId,
    EvaluationId, EventId, HttpStatusCode, InferenceStatus, OperationId, OperationKind,
    OperationStatus, PolicyAssignmentId, RecoveryId, RequestId, RequestMetadata, SessionId,
    SessionState, UsageStatus,
};

/// Provider family recorded by semantic observations.
#[allow(
    clippy::exhaustive_enums,
    reason = "OpenAI is the only provider implemented by Phase 2"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProviderKind {
    /// `OpenAI` APIs.
    OpenAi,
}

/// Versioned provider protocol recorded by semantic observations.
#[allow(
    clippy::exhaustive_enums,
    reason = "Responses API v1 is the only protocol implemented by Phase 2"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProviderProtocol {
    /// `OpenAI` Responses API v1.
    OpenAiResponsesV1,
}

/// Outcome of provider semantic observation.
#[allow(
    clippy::exhaustive_enums,
    reason = "the frozen contract fixes these forward-compatible outcomes"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ObservationStatus {
    /// Observation completed with all expected evidence.
    Complete,
    /// Observation retained only a subset of evidence.
    Partial,
    /// The input is not supported by this observer.
    Unsupported,
    /// The input was malformed.
    Malformed,
    /// A configured parser bound was reached.
    ResourceLimit,
    /// A bounded observer queue dropped semantic work.
    ObserverBackpressure,
    /// Observation was cancelled.
    Cancelled,
}

/// Provider response lifecycle state.
#[allow(
    clippy::exhaustive_enums,
    reason = "the frozen contract fixes these forward-compatible lifecycle states"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProviderResponseState {
    /// Provider accepted the response but has not started processing.
    Queued,
    /// Provider is processing the response.
    InProgress,
    /// Provider completed successfully.
    Completed,
    /// Provider stopped without a completed response.
    Incomplete,
    /// Provider reported an error.
    Failed,
    /// The client cancelled the response.
    Cancelled,
    /// The upstream disconnected before completion.
    Disconnected,
    /// No known lifecycle state was observed.
    Unknown,
}

/// Declared information-preservation class of a compression decision.
#[allow(
    clippy::exhaustive_enums,
    reason = "the plan fixes these three fidelity classes and consumers must handle each"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FidelityClass {
    /// Byte-equivalent reversible transformation.
    Exact,
    /// Equivalent data in a different representation.
    Semantic,
    /// Information was removed.
    Lossy,
}

/// One typed mutation accepted by the daemon-owned `SQLite` writer.
#[allow(
    missing_docs,
    reason = "variant field names mirror the documented canonical SQLite schema"
)]
#[derive(Debug)]
#[non_exhaustive]
pub enum WriteCommand {
    /// Inserts one session.
    Session {
        session_id: SessionId,
        started_at: String,
        ended_at: Option<String>,
        state: SessionState,
        ingress_key: String,
    },
    /// Updates one existing session lifecycle state.
    SessionState {
        session_id: SessionId,
        ended_at: Option<String>,
        state: SessionState,
    },
    /// Inserts one operation in a session.
    Operation {
        operation_id: OperationId,
        session_id: SessionId,
        kind: OperationKind,
        started_at: String,
        ended_at: Option<String>,
        status: OperationStatus,
    },
    /// Updates one existing operation lifecycle state.
    OperationState {
        operation_id: OperationId,
        ended_at: Option<String>,
        status: OperationStatus,
    },
    /// Inserts one directed operation relationship.
    CausalEdge { edge: CausalEdge },
    /// Inserts one logical provider request.
    ProviderRequest {
        operation_id: OperationId,
        metadata: RequestMetadata,
        provider: Option<ProviderKind>,
        protocol: Option<ProviderProtocol>,
        parser_version: Option<u32>,
        observation_status: Option<ObservationStatus>,
        model: Option<String>,
        stream: Option<bool>,
        background: Option<bool>,
        store: Option<bool>,
        reasoning_effort: Option<String>,
        text_verbosity: Option<String>,
        truncation: Option<String>,
        previous_response_id_present: Option<bool>,
        input_item_count: Option<u64>,
        tool_count: Option<u64>,
        text_input_block_count: Option<u64>,
        image_input_block_count: Option<u64>,
        file_input_block_count: Option<u64>,
    },
    /// Inserts one provider attempt.
    ProviderAttempt {
        attempt_id: AttemptId,
        request_id: RequestId,
        ordinal: u64,
        status_code: Option<HttpStatusCode>,
        started_at: String,
        ended_at: Option<String>,
        status: InferenceStatus,
        provider_response_id: Option<String>,
        response_model: Option<String>,
        response_state: Option<ProviderResponseState>,
        provider_created_at: Option<String>,
        incomplete_reason: Option<String>,
        error_code: Option<String>,
        transport_error: Option<String>,
        observation_status: Option<ObservationStatus>,
        streaming: Option<bool>,
        chunk_count: Option<u64>,
        byte_count: Option<u64>,
        ttfb_us: Option<u64>,
        ttft_us: Option<u64>,
        duration_us: Option<u64>,
        anomaly_metadata: Option<String>,
    },
    /// Inserts provider usage without inventing unavailable measurements.
    ProviderUsage {
        attempt_id: AttemptId,
        input_total: Option<u64>,
        input_uncached: Option<u64>,
        cache_read: Option<u64>,
        cache_write: Option<u64>,
        output_total: Option<u64>,
        reasoning: Option<u64>,
        usage_status: Option<UsageStatus>,
        raw_usage_json: Option<Box<[u8]>>,
        input_cached: Option<u64>,
        output_reasoning: Option<u64>,
        total: Option<u64>,
        normalizer_version: Option<u32>,
        anomaly_metadata: Option<String>,
    },
    /// Inserts one immutable inline content object.
    ContentObject {
        object: ContentObject,
        pin_count: u64,
        created_at: String,
    },
    /// Inserts one observation of immutable content.
    ContentOccurrence {
        occurrence: ContentOccurrence,
        observed_at: String,
    },
    /// Inserts one frozen session representation binding.
    ContentBinding { binding: ContextBinding },
    /// Inserts one compression decision without executing compression.
    CompressionDecision {
        decision_id: DecisionId,
        session_id: SessionId,
        operation_id: OperationId,
        input_content_id: ContentId,
        output_content_id: ContentId,
        fidelity: FidelityClass,
        recoverable: bool,
        compressor: String,
        compressor_version: String,
        policy_version: String,
        raw_bytes: u64,
        output_bytes: u64,
        estimated_raw_tokens: Option<u64>,
        estimated_output_tokens: Option<u64>,
        target_tokens: Option<u64>,
        latency_us: Option<u64>,
        feature_schema_version: String,
        features: Option<Box<[u8]>>,
    },
    /// Inserts one recovery mapping for a decision.
    Recovery {
        recovery_id: RecoveryId,
        decision_id: DecisionId,
        raw_content_id: ContentId,
        provenance_content_id: Option<ContentId>,
    },
    /// Inserts one session policy assignment.
    PolicyAssignment {
        policy_assignment_id: PolicyAssignmentId,
        session_id: SessionId,
        policy_version: String,
        assigned_at: String,
        chosen_action: String,
        candidate_actions: Option<Box<[u8]>>,
        action_probability: Option<f64>,
        random_seed: Option<u64>,
        feature_vector: Option<Box<[u8]>>,
    },
    /// Inserts one outcome evaluation.
    Evaluation {
        evaluation_id: EvaluationId,
        session_id: SessionId,
        operation_id: Option<OperationId>,
        provider_cost_microusd: Option<u64>,
        input_tokens: Option<u64>,
        cache_tokens: Option<u64>,
        output_tokens: Option<u64>,
        recoveries: Option<u64>,
        reruns: Option<u64>,
        latency_ms: Option<u64>,
        task_success: Option<bool>,
        user_correction: Option<bool>,
        quality_score: Option<f64>,
    },
    /// Appends one immutable event.
    Event {
        event_id: EventId,
        session_id: Option<SessionId>,
        operation_id: Option<OperationId>,
        timestamp: String,
        event_type: String,
        payload: Box<[u8]>,
        schema_version: String,
    },
}

/// A non-empty ordered set of typed commands committed in one `SQLite` transaction.
#[derive(Debug)]
pub struct WriteBatch {
    pub(crate) commands: Vec<WriteCommand>,
}

impl WriteBatch {
    /// Starts an atomic batch with its mandatory first command.
    #[must_use]
    pub fn new(first: WriteCommand) -> Self {
        Self {
            commands: vec![first],
        }
    }

    /// Appends one command while preserving transaction order.
    #[must_use]
    pub fn and(mut self, command: WriteCommand) -> Self {
        self.commands.push(command);
        self
    }
}

/// Durable result returned only after the corresponding transaction commits.
#[allow(
    clippy::exhaustive_enums,
    reason = "callers must distinguish ordinary commits from sequenced event appends"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteReceipt {
    /// A non-event mutation committed.
    Committed {
        /// Rows inserted by the command.
        rows_changed: u64,
    },
    /// An event committed at its immutable sequence.
    EventAppended {
        /// Monotonic `SQLite` event sequence.
        sequence: u64,
    },
    /// An atomic batch committed.
    BatchCommitted {
        /// Rows inserted across the batch.
        rows_changed: u64,
    },
}
