use serde::{Deserialize, Serialize};

/// A node category in the Tracepress causal operation graph.
#[allow(
    clippy::exhaustive_enums,
    reason = "the canonical operation schema requires exhaustive consumer handling"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Agent orchestration work.
    Agent,
    /// A model inference.
    LlmInference,
    /// An agent tool execution.
    ToolExecution,
    /// A content compression operation.
    Compression,
    /// A content recovery operation.
    Recovery,
    /// An outcome evaluation.
    Evaluation,
    /// A provider-native tool execution.
    ProviderTool,
    /// A provider-managed context compaction operation.
    ContextCompaction,
}

/// Persisted lifecycle state of a Tracepress session.
#[allow(
    clippy::exhaustive_enums,
    reason = "session state transitions must be handled explicitly"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// The session accepts new operations.
    Active,
    /// The session is completing an orderly shutdown.
    Closing,
    /// The session completed its shutdown.
    Closed,
    /// The daemon found a previously active session after interruption.
    Stale,
}

/// Persisted completion state of an operation.
#[allow(
    clippy::exhaustive_enums,
    reason = "operation completion must never fall through to an implied success"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// The operation has started and is not complete.
    Started,
    /// The operation completed successfully.
    Completed,
    /// The operation ended without a complete result.
    Incomplete,
    /// The operation was cancelled.
    Cancelled,
    /// The operation ended with an error.
    Errored,
    /// The operation ended because its peer disconnected.
    Disconnected,
}

/// Lifecycle state of a provider inference, including streaming outcomes.
#[allow(
    clippy::exhaustive_enums,
    reason = "PLAN.md fixes the complete inference status vocabulary"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceStatus {
    /// The inference request has started.
    Started,
    /// The provider is streaming response bytes.
    Streaming,
    /// The provider response completed.
    Completed,
    /// The provider response ended without complete output.
    Incomplete,
    /// The inference was cancelled.
    Cancelled,
    /// The inference ended with an error.
    Errored,
    /// The inference ended because a peer disconnected.
    Disconnected,
}

/// Availability and completeness of provider-reported usage.
#[allow(
    clippy::exhaustive_enums,
    reason = "PLAN.md forbids inventing usage statuses beyond these three values"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    /// The provider reported final usage.
    Final,
    /// The provider reported incomplete usage.
    Partial,
    /// The provider did not provide usage.
    Unavailable,
}
