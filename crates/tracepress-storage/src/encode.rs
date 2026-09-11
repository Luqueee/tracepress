#![allow(
    clippy::redundant_pub_crate,
    reason = "crate-private sibling modules consume these encoding functions"
)]
use rusqlite::ErrorCode;
use tracepress_core::{
    CausalRelationship, ContentKind, ContentRole, InferenceStatus, OperationKind, OperationStatus,
    RequestMethod, RequestRoute, SessionState, UsageStatus,
};

use crate::{
    AnalysisDecodeStatus, ContentEncoding, ContextAnalysisStatus, ContextBlockKind,
    ContextCorrelationStatus, ContextOrigin, ContextRole, DetectedContentKind, EstimateConfidence,
    FidelityClass, LogicalContextStatus, ObservationStatus, OpportunitySignal, ProviderKind,
    ProviderProtocol, ProviderResponseState, ProviderTransport, ReconciliationStatus, StorageError,
};

pub(crate) const fn provider_kind_text(value: ProviderKind) -> &'static str {
    match value {
        ProviderKind::OpenAi => "openai",
    }
}

pub(crate) const fn provider_protocol_text(value: ProviderProtocol) -> &'static str {
    match value {
        ProviderProtocol::OpenAiResponsesV1 => "openai-responses-v1",
    }
}

pub(crate) const fn provider_transport_text(value: ProviderTransport) -> &'static str {
    match value {
        ProviderTransport::OpenAiPublicApi => "openai_public_api",
        ProviderTransport::ChatGptCodexSubscription => "chatgpt_codex_subscription",
    }
}

pub(crate) const fn content_encoding_text(value: ContentEncoding) -> &'static str {
    match value {
        ContentEncoding::Identity => "identity",
        ContentEncoding::Zstd => "zstd",
        ContentEncoding::Unsupported => "unsupported",
    }
}

pub(crate) const fn analysis_decode_status_text(value: AnalysisDecodeStatus) -> &'static str {
    match value {
        AnalysisDecodeStatus::Identity => "identity",
        AnalysisDecodeStatus::Decoded => "decoded",
        AnalysisDecodeStatus::UnsupportedEncoding => "unsupported_encoding",
        AnalysisDecodeStatus::CorruptPayload => "corrupt_payload",
        AnalysisDecodeStatus::ResourceLimit => "resource_limit",
        AnalysisDecodeStatus::Timeout => "timeout",
    }
}

pub(crate) const fn observation_status_text(value: ObservationStatus) -> &'static str {
    match value {
        ObservationStatus::Complete => "complete",
        ObservationStatus::Partial => "partial",
        ObservationStatus::Unsupported => "unsupported",
        ObservationStatus::Malformed => "malformed",
        ObservationStatus::ResourceLimit => "resource_limit",
        ObservationStatus::ObserverBackpressure => "observer_backpressure",
        ObservationStatus::Cancelled => "cancelled",
    }
}

pub(crate) const fn response_state_text(value: ProviderResponseState) -> &'static str {
    match value {
        ProviderResponseState::Queued => "queued",
        ProviderResponseState::InProgress => "in_progress",
        ProviderResponseState::Completed => "completed",
        ProviderResponseState::Incomplete => "incomplete",
        ProviderResponseState::Failed => "failed",
        ProviderResponseState::Cancelled => "cancelled",
        ProviderResponseState::Disconnected => "disconnected",
        ProviderResponseState::Unknown => "unknown",
    }
}

pub(crate) fn sqlite_u64(value: u64, field: &'static str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_error| StorageError::IntegerOverflow { field, value })
}

pub(crate) fn sqlite_optional(
    value: Option<u64>,
    field: &'static str,
) -> Result<Option<i64>, StorageError> {
    value.map(|inner| sqlite_u64(inner, field)).transpose()
}

pub(crate) fn sqlite<T>(result: rusqlite::Result<T>) -> Result<T, StorageError> {
    result.map_err(|error| match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StorageError::Busy(error),
        Some(_) | None => StorageError::Sqlite(error),
    })
}

pub(crate) const fn session_state(value: SessionState) -> &'static str {
    match value {
        SessionState::Active => "active",
        SessionState::Closing => "closing",
        SessionState::Closed => "closed",
        SessionState::Stale => "stale",
    }
}

pub(crate) const fn operation_kind(value: OperationKind) -> &'static str {
    match value {
        OperationKind::Agent => "agent",
        OperationKind::LlmInference => "llm_inference",
        OperationKind::ToolExecution => "tool_execution",
        OperationKind::Compression => "compression",
        OperationKind::Recovery => "recovery",
        OperationKind::Evaluation => "evaluation",
        OperationKind::ProviderTool => "provider_tool",
    }
}

pub(crate) const fn operation_status(value: OperationStatus) -> &'static str {
    match value {
        OperationStatus::Started => "started",
        OperationStatus::Completed => "completed",
        OperationStatus::Incomplete => "incomplete",
        OperationStatus::Cancelled => "cancelled",
        OperationStatus::Errored => "errored",
        OperationStatus::Disconnected => "disconnected",
    }
}

pub(crate) const fn inference_status(value: InferenceStatus) -> &'static str {
    match value {
        InferenceStatus::Started => "started",
        InferenceStatus::Streaming => "streaming",
        InferenceStatus::Completed => "completed",
        InferenceStatus::Incomplete => "incomplete",
        InferenceStatus::Cancelled => "cancelled",
        InferenceStatus::Errored => "errored",
        InferenceStatus::Disconnected => "disconnected",
    }
}

pub(crate) const fn request_route(value: RequestRoute) -> &'static str {
    match value {
        RequestRoute::ChatCompletions => "chat_completions",
        RequestRoute::Responses => "responses",
    }
}

pub(crate) const fn request_method(value: RequestMethod) -> &'static str {
    match value {
        RequestMethod::Post => "post",
    }
}

pub(crate) const fn usage_status_text(value: UsageStatus) -> &'static str {
    match value {
        UsageStatus::Final => "final",
        UsageStatus::Partial => "partial",
        UsageStatus::Unavailable => "unavailable",
    }
}

pub(crate) const fn causal_relationship(value: CausalRelationship) -> &'static str {
    match value {
        CausalRelationship::Spawned => "spawned",
        CausalRelationship::FollowsFrom => "follows_from",
        CausalRelationship::RecoveryOf => "recovery_of",
        CausalRelationship::EvaluationOf => "evaluation_of",
    }
}

pub(crate) const fn content_kind(value: ContentKind) -> &'static str {
    match value {
        ContentKind::Text => "text",
        ContentKind::Json => "json",
        ContentKind::Ndjson => "ndjson",
        ContentKind::Log => "log",
        ContentKind::SearchResults => "search_results",
        ContentKind::TestResults => "test_results",
        ContentKind::SourceCode => "source_code",
        ContentKind::Diff => "diff",
        ContentKind::Image => "image",
        ContentKind::Document => "document",
        ContentKind::Binary => "binary",
        ContentKind::Unknown => "unknown",
    }
}

pub(crate) const fn content_role(value: ContentRole) -> &'static str {
    match value {
        ContentRole::ProcessOutput => "process_output",
        ContentRole::AgentToolResult => "agent_tool_result",
        ContentRole::TracepressResult => "tracepress_result",
        ContentRole::ProviderInput => "provider_input",
    }
}

pub(crate) const fn fidelity_class(value: FidelityClass) -> &'static str {
    match value {
        FidelityClass::Exact => "exact",
        FidelityClass::Semantic => "semantic",
        FidelityClass::Lossy => "lossy",
    }
}

pub(crate) const fn context_analysis_status_text(value: ContextAnalysisStatus) -> &'static str {
    match value {
        ContextAnalysisStatus::Complete => "complete",
        ContextAnalysisStatus::Partial => "partial",
        ContextAnalysisStatus::ResourceLimit => "resource_limit",
        ContextAnalysisStatus::Malformed => "malformed",
        ContextAnalysisStatus::ObserverBackpressure => "observer_backpressure",
        ContextAnalysisStatus::CorrelationDegraded => "correlation_degraded",
        ContextAnalysisStatus::Unsupported => "unsupported",
        ContextAnalysisStatus::Cancelled => "cancelled",
    }
}

pub(crate) const fn logical_context_status_text(value: LogicalContextStatus) -> &'static str {
    match value {
        LogicalContextStatus::ExplicitOnly => "explicit_only",
        LogicalContextStatus::ProviderManagedPartial => "provider_managed_partial",
        LogicalContextStatus::ExternalReferencesPartial => "external_references_partial",
        LogicalContextStatus::MixedPartial => "mixed_partial",
        LogicalContextStatus::Unknown => "unknown",
    }
}

pub(crate) const fn context_correlation_status_text(
    value: ContextCorrelationStatus,
) -> &'static str {
    match value {
        ContextCorrelationStatus::Correlated => "correlated",
        ContextCorrelationStatus::Degraded => "degraded",
    }
}

pub(crate) const fn context_block_kind_text(value: ContextBlockKind) -> &'static str {
    match value {
        ContextBlockKind::Instructions => "instructions",
        ContextBlockKind::Message => "message",
        ContextBlockKind::Text => "text",
        ContextBlockKind::ImageReference => "image_reference",
        ContextBlockKind::FileReference => "file_reference",
        ContextBlockKind::ToolDefinition => "tool_definition",
        ContextBlockKind::ToolCall => "tool_call",
        ContextBlockKind::ToolResult => "tool_result",
        ContextBlockKind::ItemReference => "item_reference",
        ContextBlockKind::PromptReference => "prompt_reference",
        ContextBlockKind::ProviderStateReference => "provider_state_reference",
        ContextBlockKind::AssistantHistory => "assistant_history",
        ContextBlockKind::OpaqueReasoning => "opaque_reasoning",
        ContextBlockKind::Opaque => "opaque",
        ContextBlockKind::Unknown => "unknown",
    }
}

pub(crate) const fn context_role_text(value: ContextRole) -> &'static str {
    match value {
        ContextRole::System => "system",
        ContextRole::Developer => "developer",
        ContextRole::User => "user",
        ContextRole::Assistant => "assistant",
        ContextRole::Tool => "tool",
        ContextRole::Unknown => "unknown",
    }
}

pub(crate) const fn context_origin_text(value: ContextOrigin) -> &'static str {
    match value {
        ContextOrigin::HumanAuthored => "human_authored",
        ContextOrigin::AgentGenerated => "agent_generated",
        ContextOrigin::ToolGenerated => "tool_generated",
        ContextOrigin::ToolSchema => "tool_schema",
        ContextOrigin::ProviderManaged => "provider_managed",
        ContextOrigin::ExternalReference => "external_reference",
        ContextOrigin::TracepressGenerated => "tracepress_generated",
        ContextOrigin::Unknown => "unknown",
    }
}

pub(crate) const fn estimate_confidence_text(value: EstimateConfidence) -> &'static str {
    match value {
        EstimateConfidence::ModelMapped => "model_mapped",
        EstimateConfidence::GenericTokenizer => "generic_tokenizer",
        EstimateConfidence::Heuristic => "heuristic",
    }
}

pub(crate) const fn reconciliation_status_text(value: ReconciliationStatus) -> &'static str {
    match value {
        ReconciliationStatus::ComparableApproximate => "comparable_approximate",
        ReconciliationStatus::PartialVisibility => "partial_visibility",
        ReconciliationStatus::MissingProviderUsage => "missing_provider_usage",
        ReconciliationStatus::MissingLocalEstimate => "missing_local_estimate",
        ReconciliationStatus::NotComparable => "not_comparable",
    }
}

pub(crate) const fn detected_content_kind_text(value: DetectedContentKind) -> &'static str {
    match value {
        DetectedContentKind::Json => "json",
        DetectedContentKind::Ndjson => "ndjson",
        DetectedContentKind::Log => "log",
        DetectedContentKind::SearchResults => "search_results",
        DetectedContentKind::TestResults => "test_results",
        DetectedContentKind::SourceCode => "source_code",
        DetectedContentKind::Diff => "diff",
        DetectedContentKind::PlainText => "plain_text",
        DetectedContentKind::BinaryLike => "binary_like",
        DetectedContentKind::Unknown => "unknown",
    }
}

const fn opportunity_signal_text(value: OpportunitySignal) -> &'static str {
    match value {
        OpportunitySignal::LargeToolResult => "large_tool_result",
        OpportunitySignal::HighDuplication => "high_duplication",
        OpportunitySignal::HomogeneousJson => "homogeneous_json",
        OpportunitySignal::RepetitiveLogs => "repetitive_logs",
        OpportunitySignal::LargeSearchResult => "large_search_result",
        OpportunitySignal::LargeTestOutput => "large_test_output",
        OpportunitySignal::LargeToolSchema => "large_tool_schema",
        OpportunitySignal::RepeatedHistory => "repeated_history",
    }
}

/// Encodes a signal set canonically: sorted, deduplicated, and comma separated.
///
/// An empty slice encodes as an empty string, which records that detection ran and
/// raised nothing; a `NULL` column records that no signal was ever evaluated.
pub(crate) fn opportunity_signals_text(signals: &[OpportunitySignal]) -> String {
    let mut names: Vec<&'static str> = signals
        .iter()
        .copied()
        .map(opportunity_signal_text)
        .collect();
    names.sort_unstable();
    names.dedup();
    names.join(",")
}
