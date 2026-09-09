use tracepress_core::{
    AttemptId, CausalEdge, ContentId, ContentObject, ContentOccurrence, ContextBinding,
    ContextBlockOccurrenceId, ContextSnapshotId, DecisionId, EvaluationId, EventId, HttpStatusCode,
    InferenceStatus, OperationId, OperationKind, OperationStatus, PolicyAssignmentId, RecoveryId,
    RequestId, RequestMetadata, SessionId, SessionState, UsageStatus,
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

/// Terminal outcome of one shadow context analysis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContextAnalysisStatus {
    /// Every eligible block was analyzed and the analysis was finalized.
    Complete,
    /// Useful aggregates exist, but the analysis is not complete.
    Partial,
    /// A configured analysis bound stopped part of the work.
    ResourceLimit,
    /// The observed request bytes could not be parsed.
    Malformed,
    /// A bounded observer queue dropped the analysis.
    ObserverBackpressure,
    /// Correlation degradation prevented a trustworthy analysis.
    CorrelationDegraded,
    /// The observed protocol is not context-analysed.
    Unsupported,
    /// The analysis was cancelled.
    Cancelled,
}

/// How completely Tracepress can account for the logical context of one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LogicalContextStatus {
    /// Every contributing byte was explicit in the observed request.
    ExplicitOnly,
    /// Provider-managed state contributes context Tracepress cannot observe.
    ProviderManagedPartial,
    /// External references contribute context Tracepress does not fetch.
    ExternalReferencesPartial,
    /// Both provider-managed state and external references contribute.
    MixedPartial,
    /// Visibility could not be determined.
    Unknown,
}

/// Whether a snapshot's causal correlation was complete or degraded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContextCorrelationStatus {
    /// The request halves and their inference operation were fully correlated.
    Correlated,
    /// A bounded correlation table lost part of the causal link.
    Degraded,
}

/// Structural category of one observed context block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContextBlockKind {
    /// Top-level request instructions.
    Instructions,
    /// A conversation message item.
    Message,
    /// A text content part.
    Text,
    /// A reference to an image.
    ImageReference,
    /// A reference to a file.
    FileReference,
    /// A tool or function schema declaration.
    ToolDefinition,
    /// A model-issued tool call.
    ToolCall,
    /// A tool or function call output.
    ToolResult,
    /// A reference to a provider-held item.
    ItemReference,
    /// A reference to a provider-held prompt.
    PromptReference,
    /// A reference to provider-managed conversation state.
    ProviderStateReference,
    /// Prior assistant output replayed as history.
    AssistantHistory,
    /// Opaque reasoning content.
    OpaqueReasoning,
    /// Opaque content of an unstated shape.
    Opaque,
    /// No known category was recognised.
    Unknown,
}

/// Conversational role of one observed context block, orthogonal to its kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContextRole {
    /// System role.
    System,
    /// Developer role.
    Developer,
    /// End-user role.
    User,
    /// Assistant role.
    Assistant,
    /// Tool role.
    Tool,
    /// No known role was recognised.
    Unknown,
}

/// Structurally observed producer of one context block, recorded but never acted on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContextOrigin {
    /// Authored by a human.
    HumanAuthored,
    /// Generated by the agent or model.
    AgentGenerated,
    /// Produced by a tool execution.
    ToolGenerated,
    /// Declared as a tool schema.
    ToolSchema,
    /// Held by the provider.
    ProviderManaged,
    /// Named by an external reference.
    ExternalReference,
    /// Produced by Tracepress itself.
    TracepressGenerated,
    /// No known origin was recognised.
    Unknown,
}

/// Confidence class of one token estimate; no estimate is ever exact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EstimateConfidence {
    /// A tokenizer mapped from the requested model produced the estimate.
    ModelMapped,
    /// A generic tokenizer produced the estimate.
    GenericTokenizer,
    /// A structural heuristic produced the estimate.
    Heuristic,
}

/// Comparability of a local estimate against provider-reported input tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReconciliationStatus {
    /// Both values exist over fully explicit context and are approximately comparable.
    ComparableApproximate,
    /// Both values exist, but part of the logical context is invisible.
    PartialVisibility,
    /// No provider usage was observed.
    MissingProviderUsage,
    /// No local estimate could be produced.
    MissingLocalEstimate,
    /// The two values cannot be compared.
    NotComparable,
}

/// Structurally detected shape of block content; detection activates no transformation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DetectedContentKind {
    /// A single JSON document.
    Json,
    /// Newline-delimited JSON.
    Ndjson,
    /// Log output.
    Log,
    /// Search or grep results.
    SearchResults,
    /// Test-runner output.
    TestResults,
    /// Source code.
    SourceCode,
    /// A unified diff.
    Diff,
    /// Plain text.
    PlainText,
    /// Binary-like bytes.
    BinaryLike,
    /// Every detector abstained.
    Unknown,
}

/// A recorded signal, never a decision and never a saving claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OpportunitySignal {
    /// A tool result dominates the observed context.
    LargeToolResult,
    /// Content repeats heavily within the observed context.
    HighDuplication,
    /// JSON items share a homogeneous shape.
    HomogeneousJson,
    /// Log lines repeat with low variation.
    RepetitiveLogs,
    /// A search result set is large.
    LargeSearchResult,
    /// Test output is large.
    LargeTestOutput,
    /// A tool schema is large.
    LargeToolSchema,
    /// History repeats across requests in the session.
    RepeatedHistory,
}

const fn context_block_kind_index(value: ContextBlockKind) -> usize {
    match value {
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
        ContextBlockKind::Unknown => 14,
    }
}

const fn context_role_index(value: ContextRole) -> usize {
    match value {
        ContextRole::System => 0,
        ContextRole::Developer => 1,
        ContextRole::User => 2,
        ContextRole::Assistant => 3,
        ContextRole::Tool => 4,
        ContextRole::Unknown => 5,
    }
}

const fn context_origin_index(value: ContextOrigin) -> usize {
    match value {
        ContextOrigin::HumanAuthored => 0,
        ContextOrigin::AgentGenerated => 1,
        ContextOrigin::ToolGenerated => 2,
        ContextOrigin::ToolSchema => 3,
        ContextOrigin::ProviderManaged => 4,
        ContextOrigin::ExternalReference => 5,
        ContextOrigin::TracepressGenerated => 6,
        ContextOrigin::Unknown => 7,
    }
}

/// Estimated tokens attributed to each context block kind, `None` where unknown.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EstimatedTokensByKind([Option<u64>; 15]);

impl EstimatedTokensByKind {
    /// Creates a breakdown in which every kind is unknown.
    #[must_use]
    pub const fn new() -> Self {
        Self([None; 15])
    }

    /// Attributes an estimated token count to one kind.
    #[must_use]
    pub fn with(mut self, kind: ContextBlockKind, estimated_tokens: u64) -> Self {
        if let Some(slot) = self.0.get_mut(context_block_kind_index(kind)) {
            *slot = Some(estimated_tokens);
        }
        self
    }

    /// Returns the estimated tokens attributed to one kind, or `None` where unknown.
    #[must_use]
    pub fn get(&self, kind: ContextBlockKind) -> Option<u64> {
        self.0
            .get(context_block_kind_index(kind))
            .copied()
            .flatten()
    }
}

/// Estimated tokens attributed to each context role, `None` where unknown.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EstimatedTokensByRole([Option<u64>; 6]);

impl EstimatedTokensByRole {
    /// Creates a breakdown in which every role is unknown.
    #[must_use]
    pub const fn new() -> Self {
        Self([None; 6])
    }

    /// Attributes an estimated token count to one role.
    #[must_use]
    pub fn with(mut self, role: ContextRole, estimated_tokens: u64) -> Self {
        if let Some(slot) = self.0.get_mut(context_role_index(role)) {
            *slot = Some(estimated_tokens);
        }
        self
    }

    /// Returns the estimated tokens attributed to one role, or `None` where unknown.
    #[must_use]
    pub fn get(&self, role: ContextRole) -> Option<u64> {
        self.0.get(context_role_index(role)).copied().flatten()
    }
}

/// Estimated tokens attributed to each context origin, `None` where unknown.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EstimatedTokensByOrigin([Option<u64>; 8]);

impl EstimatedTokensByOrigin {
    /// Creates a breakdown in which every origin is unknown.
    #[must_use]
    pub const fn new() -> Self {
        Self([None; 8])
    }

    /// Attributes an estimated token count to one origin.
    #[must_use]
    pub fn with(mut self, origin: ContextOrigin, estimated_tokens: u64) -> Self {
        if let Some(slot) = self.0.get_mut(context_origin_index(origin)) {
            *slot = Some(estimated_tokens);
        }
        self
    }

    /// Returns the estimated tokens attributed to one origin, or `None` where unknown.
    #[must_use]
    pub fn get(&self, origin: ContextOrigin) -> Option<u64> {
        self.0.get(context_origin_index(origin)).copied().flatten()
    }
}

/// Estimated tokens grouped by kind, role, and origin for one snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EstimatedTokenComposition {
    kinds: EstimatedTokensByKind,
    roles: EstimatedTokensByRole,
    origins: EstimatedTokensByOrigin,
}

impl EstimatedTokenComposition {
    /// Groups the three estimated-token breakdowns of one snapshot.
    #[must_use]
    pub const fn new(
        kinds: EstimatedTokensByKind,
        roles: EstimatedTokensByRole,
        origins: EstimatedTokensByOrigin,
    ) -> Self {
        Self {
            kinds,
            roles,
            origins,
        }
    }

    /// Returns the estimated tokens grouped by block kind.
    #[must_use]
    pub const fn by_kind(&self) -> EstimatedTokensByKind {
        self.kinds
    }

    /// Returns the estimated tokens grouped by role.
    #[must_use]
    pub const fn by_role(&self) -> EstimatedTokensByRole {
        self.roles
    }

    /// Returns the estimated tokens grouped by origin.
    #[must_use]
    pub const fn by_origin(&self) -> EstimatedTokensByOrigin {
        self.origins
    }
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
    /// Inserts one shadow context analysis snapshot when analysis begins.
    ///
    /// The inserted `status` is never `Complete`: a snapshot only becomes complete
    /// through `ContextSnapshotOutcome` once its finalization commits.
    ContextSnapshot {
        snapshot_id: ContextSnapshotId,
        session_id: SessionId,
        provider_request_id: RequestId,
        inference_operation_id: OperationId,
        analysis_version: u32,
        status: ContextAnalysisStatus,
        started_at_us: u64,
    },
    /// Records the terminal status, visibility, and totals of one snapshot.
    ContextSnapshotOutcome {
        snapshot_id: ContextSnapshotId,
        status: ContextAnalysisStatus,
        completed_at_us: Option<u64>,
        request_content_hash: Option<Box<[u8]>>,
        explicit_block_count: Option<u64>,
        analyzed_bytes: Option<u64>,
        skipped_bytes: Option<u64>,
        explicit_request_complete: Option<bool>,
        uses_previous_response: Option<bool>,
        uses_conversation_state: Option<bool>,
        uses_item_references: Option<bool>,
        uses_prompt_reference: Option<bool>,
        uses_external_files: Option<bool>,
        uses_external_images: Option<bool>,
        contains_opaque_items: Option<bool>,
        logical_context_status: Option<LogicalContextStatus>,
        duplicate_key_detected: Option<bool>,
        reference_resolved_locally: Option<bool>,
        correlation_status: Option<ContextCorrelationStatus>,
    },
    /// Inserts one positioned, measured context block of a snapshot.
    ContextBlockOccurrence {
        block_occurrence_id: ContextBlockOccurrenceId,
        snapshot_id: ContextSnapshotId,
        ordinal: u64,
        parent_block_occurrence_id: Option<ContextBlockOccurrenceId>,
        kind: ContextBlockKind,
        role: ContextRole,
        origin: ContextOrigin,
        semantic_path: Option<String>,
        semantic_path_truncated: Option<bool>,
        semantic_path_hash: Option<Box<[u8]>>,
        raw_value_start: u64,
        raw_value_end: u64,
        locator_occurrence: u64,
        raw_bytes: u64,
        exact_fingerprint: Option<Box<[u8]>>,
        semantic_fingerprint: Option<Box<[u8]>>,
        fingerprint_version: Option<u32>,
        estimated_tokens: Option<u64>,
        estimator: Option<String>,
        estimator_version: Option<u32>,
        estimator_encoding: Option<String>,
        estimate_confidence: Option<EstimateConfidence>,
        detected_kind: Option<DetectedContentKind>,
        detector_confidence: Option<f64>,
        detector_version: Option<u32>,
        tool_call_id: Option<String>,
        tool_name: Option<String>,
        tool_name_truncated: Option<bool>,
        tool_name_hash: Option<Box<[u8]>>,
        line_count: Option<u64>,
        max_line_length: Option<u64>,
        duplicate_line_ratio: Option<f64>,
        unique_line_ratio: Option<f64>,
        json_item_count: Option<u64>,
        json_depth: Option<u64>,
        error_line_density: Option<f64>,
        warning_line_density: Option<f64>,
        repetition_score: Option<f64>,
        opportunity_signals: Option<Box<[OpportunitySignal]>>,
        candidate_estimated_tokens: Option<u64>,
    },
    /// Inserts the composition aggregates of one snapshot.
    ContextAnalysisMetrics {
        snapshot_id: ContextSnapshotId,
        explicit_bytes: Option<u64>,
        estimated_tokens: Box<EstimatedTokenComposition>,
        estimated_tool_definition_share: Option<f64>,
        estimated_tool_result_share: Option<f64>,
        estimated_human_text_share: Option<f64>,
        estimated_assistant_history_share: Option<f64>,
        estimated_unique_content_share: Option<f64>,
        estimated_repeated_content_share: Option<f64>,
        tool_count: Option<u64>,
        schema_bytes: Option<u64>,
        estimated_schema_tokens: Option<u64>,
        largest_tool_schema: Option<u64>,
        repeated_schema_tokens: Option<u64>,
        stable_explicit_prefix_estimate: Option<u64>,
        estimator: Option<String>,
        estimator_version: Option<u32>,
        estimate_confidence: Option<EstimateConfidence>,
        opportunity_signals: Option<Box<[OpportunitySignal]>>,
    },
    /// Inserts the repetition delta between one snapshot and its predecessor.
    ContextDelta {
        current_snapshot_id: ContextSnapshotId,
        previous_snapshot_id: ContextSnapshotId,
        repeated_blocks: Option<u64>,
        new_blocks: Option<u64>,
        changed_blocks: Option<u64>,
        removed_blocks: Option<u64>,
        repeated_estimated_tokens: Option<u64>,
        new_estimated_tokens: Option<u64>,
        common_prefix_blocks: Option<u64>,
        common_prefix_estimated_tokens: Option<u64>,
    },
    /// Inserts one comparison of a local estimate against provider-reported input tokens.
    ///
    /// `residual_tokens` is signed and is stored exactly as computed; a negative
    /// residual is estimator-accuracy evidence and is never clamped.
    TokenReconciliation {
        snapshot_id: ContextSnapshotId,
        attempt_id: Option<AttemptId>,
        visible_estimated_tokens: Option<u64>,
        provider_input_tokens: Option<u64>,
        residual_tokens: Option<i64>,
        comparability: ReconciliationStatus,
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
