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

/// Transport surface recorded for a provider request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProviderTransport {
    /// `OpenAI`'s public API.
    OpenAiPublicApi,
    /// `ChatGPT` Codex subscription backend.
    ChatGptCodexSubscription,
}

/// Content encoding recorded for the analysis-only request decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContentEncoding {
    /// No encoding or identity.
    Identity,
    /// Zstandard.
    Zstd,
    /// Unsupported or ambiguous content encoding.
    Unsupported,
}

/// Outcome recorded by the analysis-only decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AnalysisDecodeStatus {
    /// Analyzer used identity bytes.
    Identity,
    /// Analyzer used bounded decoded bytes.
    Decoded,
    /// Encoding was unsupported or ambiguous.
    UnsupportedEncoding,
    /// Encoded body was invalid or truncated.
    CorruptPayload,
    /// Decode byte/work bound was reached.
    ResourceLimit,
    /// Decode time bound was reached.
    Timeout,
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

/// Bounded lifecycle status for one persisted context snapshot.
///
/// This projection intentionally excludes block and aggregate data. The daemon uses it while
/// reconciling an interrupted observer, where the only durable truth needed is whether the
/// snapshot committed a terminal outcome.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextSnapshotStatus {
    /// Durable snapshot identity.
    pub snapshot_id: ContextSnapshotId,
    /// Provider request identity associated with the snapshot.
    pub provider_request_id: RequestId,
    /// Stable storage status name.
    pub status: String,
    /// Commit timestamp for a terminal outcome, when one was written.
    pub completed_at_us: Option<u64>,
}
impl ContextSnapshotStatus {
    /// Constructs one durable context snapshot lifecycle status without a commit timestamp.
    #[must_use]
    pub const fn new(
        snapshot_id: ContextSnapshotId,
        provider_request_id: RequestId,
        status: String,
    ) -> Self {
        Self {
            snapshot_id,
            provider_request_id,
            status,
            completed_at_us: None,
        }
    }

    /// Adds the durable commit timestamp to this lifecycle status.
    #[must_use]
    pub const fn with_completed_at(mut self, completed_at_us: Option<u64>) -> Self {
        self.completed_at_us = completed_at_us;
        self
    }
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

/// Maximum number of context blocks returned by one metadata inspection.
///
/// The bound is part of the read contract: a large request never expands an IPC response with
/// one row per block.
pub const CONTEXT_INSPECTION_MAX_BLOCKS: usize = 10;

/// A bounded, metadata-only view of one persisted context snapshot.
///
/// Values that were not observed remain `None`; this type never substitutes zero for missing
/// evidence. String fields are canonical enum names or bounded provider metadata, never request
/// content, tool arguments, paths, or fingerprints.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextInspection {
    /// Logical provider request identity selected by the query.
    pub request_id: RequestId,
    /// Durable context snapshot identity.
    pub snapshot_id: ContextSnapshotId,
    /// Session that owns the request.
    pub session_id: SessionId,
    /// Inference operation that produced the request.
    pub inference_operation_id: OperationId,
    /// Provider attempt used for reconciliation, when one was recorded.
    pub attempt_id: Option<AttemptId>,
    /// Provider family, when observed.
    pub provider: Option<String>,
    /// Provider protocol, when observed.
    pub protocol: Option<String>,
    /// Accepted request byte count.
    pub request_bytes: Option<u64>,
    /// Provider model identifier, bounded before it crosses IPC.
    pub model: Option<String>,
    /// Context analyzer version.
    pub analysis_version: u32,
    /// Context analyzer terminal status.
    pub analysis_status: String,
    /// Context-to-provider correlation state, when recorded.
    pub correlation_status: Option<String>,
    /// Visibility and logical-context state.
    pub visibility: ContextInspectionVisibility,
    /// Provider-reported input tokens, when observed.
    pub provider_input_tokens: Option<u64>,
    /// Local visible-context estimate.
    pub visible_estimated_tokens: Option<u64>,
    /// Estimator identifier.
    pub estimator: Option<String>,
    /// Estimator version.
    pub estimator_version: Option<u32>,
    /// Confidence class for the local estimate.
    pub estimate_confidence: Option<String>,
    /// Reconciliation comparability status.
    pub reconciliation_status: Option<String>,
    /// Signed residual (`provider_input_tokens - visible_estimated_tokens`).
    pub residual_tokens: Option<i64>,
    /// Bounded token composition and named shares.
    pub composition: ContextInspectionComposition,
    /// Cross-snapshot repetition metrics.
    pub repetition: ContextInspectionRepetition,
    /// Stable explicit-prefix estimate, when available.
    pub stable_explicit_prefix_estimate: Option<u64>,
    /// Analysis coverage and visibility evidence.
    pub coverage: ContextInspectionCoverage,
    /// Largest blocks by raw bytes, bounded to [`CONTEXT_INSPECTION_MAX_BLOCKS`].
    pub largest_blocks: Vec<ContextInspectionBlock>,
}

/// Visibility metadata for one context inspection.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextInspectionVisibility {
    /// Whether every request byte was explicit to Tracepress.
    pub explicit_request_complete: Option<bool>,
    /// Whether the request reuses a previous provider response.
    pub uses_previous_response: Option<bool>,
    /// Whether provider conversation state contributes context.
    pub uses_conversation_state: Option<bool>,
    /// Whether provider-held item references contribute context.
    pub uses_item_references: Option<bool>,
    /// Whether a provider-held prompt reference contributes context.
    pub uses_prompt_reference: Option<bool>,
    /// Whether external files contribute context.
    pub uses_external_files: Option<bool>,
    /// Whether external images contribute context.
    pub uses_external_images: Option<bool>,
    /// Whether opaque items were observed.
    pub contains_opaque_items: Option<bool>,
    /// Logical accounting status.
    pub logical_context_status: Option<String>,
    /// Whether duplicate object keys were observed.
    pub duplicate_key_detected: Option<bool>,
    /// Whether references were resolved locally.
    pub reference_resolved_locally: Option<bool>,
}

/// A named token estimate in a composition breakdown.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextInspectionNamedEstimate {
    /// Stable enum name.
    pub name: String,
    /// Estimated tokens attributed to this name.
    pub estimated_tokens: Option<u64>,
}

/// Bounded token composition for a context snapshot.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextInspectionComposition {
    /// Estimates grouped by structural block kind.
    pub by_kind: Vec<ContextInspectionNamedEstimate>,
    /// Estimates grouped by conversational role.
    pub by_role: Vec<ContextInspectionNamedEstimate>,
    /// Estimates grouped by producer origin.
    pub by_origin: Vec<ContextInspectionNamedEstimate>,
    /// Share of estimated tokens in tool definitions.
    pub estimated_tool_definition_share: Option<f64>,
    /// Share of estimated tokens in tool results.
    pub estimated_tool_result_share: Option<f64>,
    /// Share of estimated tokens in human-authored text.
    pub estimated_human_text_share: Option<f64>,
    /// Share of estimated tokens in assistant history.
    pub estimated_assistant_history_share: Option<f64>,
    /// Share of estimated tokens in unique content.
    pub estimated_unique_content_share: Option<f64>,
    /// Share of estimated tokens in repeated content.
    pub estimated_repeated_content_share: Option<f64>,
    /// Number of tool definitions observed.
    pub tool_count: Option<u64>,
    /// Bytes occupied by tool schemas.
    pub schema_bytes: Option<u64>,
    /// Estimated tokens in tool schemas.
    pub estimated_schema_tokens: Option<u64>,
    /// Largest tool schema in bytes.
    pub largest_tool_schema: Option<u64>,
    /// Estimated tokens in repeated tool schemas.
    pub repeated_schema_tokens: Option<u64>,
    /// Signals raised across analyzed blocks.
    pub opportunity_signals: Vec<String>,
}

/// Cross-snapshot repetition and stable-prefix metrics.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextInspectionRepetition {
    /// Number of blocks repeated from the previous snapshot.
    pub repeated_blocks: Option<u64>,
    /// Number of new blocks.
    pub new_blocks: Option<u64>,
    /// Number of changed blocks.
    pub changed_blocks: Option<u64>,
    /// Number of removed blocks.
    pub removed_blocks: Option<u64>,
    /// Estimated tokens in repeated blocks.
    pub repeated_estimated_tokens: Option<u64>,
    /// Estimated tokens in new blocks.
    pub new_estimated_tokens: Option<u64>,
    /// Stable common-prefix block count.
    pub common_prefix_blocks: Option<u64>,
    /// Estimated tokens in the stable common prefix.
    pub common_prefix_estimated_tokens: Option<u64>,
}

/// Coverage fields retained by bounded context analysis.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextInspectionCoverage {
    /// Number of explicit blocks retained.
    pub explicit_block_count: Option<u64>,
    /// Bytes admitted to analysis.
    pub analyzed_bytes: Option<u64>,
    /// Bytes skipped by analysis bounds.
    pub skipped_bytes: Option<u64>,
    /// Explicit bytes reported by aggregate metrics.
    pub explicit_bytes: Option<u64>,
}

/// Metadata for one of the largest context blocks.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ContextInspectionBlock {
    /// Stable position in the request.
    pub ordinal: u64,
    /// Structural block kind.
    pub kind: String,
    /// Conversational role.
    pub role: String,
    /// Producer origin.
    pub origin: String,
    /// Raw bytes occupied by the block.
    pub raw_bytes: u64,
    /// Local estimate for this block.
    pub estimated_tokens: Option<u64>,
    /// Detector result, when available.
    pub detected_kind: Option<String>,
    /// Detector confidence, when available.
    pub detector_confidence: Option<f64>,
    /// Detector implementation version, when available.
    pub detector_version: Option<u32>,
    /// Structural opportunity signals only.
    pub opportunity_signals: Vec<String>,
    /// Existing estimate attached to a raised opportunity signal.
    pub candidate_estimated_tokens: Option<u64>,
    /// Bounded repetition score.
    pub repetition_score: Option<f64>,
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
        transport: Option<ProviderTransport>,
        endpoint_profile_version: Option<u32>,
        content_encoding: Option<ContentEncoding>,
        analysis_decode_status: Option<AnalysisDecodeStatus>,
        wire_bytes: Option<u64>,
        wire_sha256: Option<Box<[u8]>>,
        decoded_bytes: Option<u64>,
        decode_duration_us: Option<u64>,
        decoder_version: Option<u32>,
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
        analysis_content_hash: Option<Box<[u8]>>,
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

/// Counts the durable state transitions performed during startup recovery.
///
/// Session transitions and unfinished context snapshots are reported separately because snapshot
/// and event rows are auxiliary effects of recovering a session, not additional sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RecoveryReceipt {
    /// Number of active or closing sessions marked stale.
    pub recovered_sessions: u64,
    /// Number of unfinished context snapshots marked partial.
    pub recovered_context_snapshots: u64,
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
