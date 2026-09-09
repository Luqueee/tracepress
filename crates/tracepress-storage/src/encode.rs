#![allow(
    clippy::redundant_pub_crate,
    reason = "crate-private sibling modules consume these encoding functions"
)]

use rusqlite::ErrorCode;
use tracepress_core::{
    CausalRelationship, ContentKind, ContentRole, InferenceStatus, OperationKind, OperationStatus,
    RequestMethod, RequestRoute, SessionState, UsageStatus,
};

use crate::{FidelityClass, StorageError};

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
