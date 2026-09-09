//! Shared domain types and invariants for Tracepress.

mod binding;
mod byte_boundary;
mod causal;
mod content;
mod content_id;
mod finalization;
mod ids;
mod lifecycle;
mod limits;
mod occurrence;
mod request;
mod response_stream;
mod structural_budget;

pub use binding::{
    BindingCandidate, BindingConflictField, BindingKey, BindingRepresentation, BindingVersions,
    ContextBinding, ContextBindingError,
};
pub use byte_boundary::{
    ByteBoundaryError, ByteDecision, ByteLimitExceeded, ByteResource, DeclaredLengthDecision,
    TruncationMetadata, classify_decompressed_length, classify_ipc_frame, classify_line_bytes,
    classify_raw_bytes, classify_request_body,
};
pub use causal::{CausalEdge, CausalEdgeError, CausalRelationship};
pub use content::{ContentKind, ContentObject, ContentObjectError, RawContent};
pub use content_id::{ContentId, ContentIdParseError};
pub use finalization::{CommitState, FinalizationError, finalize_inference};
pub use ids::{
    AttemptId, BindingId, ContextBlockOccurrenceId, ContextSnapshotId, DecisionId, EvaluationId,
    EventId, IdParseError, OccurrenceId, OperationId, PolicyAssignmentId, RecoveryId, RequestId,
    SessionId, ToolCallId, UuidV7Generator, UuidV7Timestamp, UuidV7TimestampError,
};
pub use lifecycle::{InferenceStatus, OperationKind, OperationStatus, SessionState, UsageStatus};
pub use limits::{
    LimitValueError, MaxCpuWorkUnits, MaxDecompressedBytes, MaxIpcFrameBytes, MaxIpcQueueItems,
    MaxJsonItems, MaxJsonNesting, MaxLineBytes, MaxProcessingTimeMs, MaxRawBytes,
    MaxRequestBodyBytes, MaxResponseBodyBytes, ResourceLimitField, ResourceLimits,
    ResourceLimitsConfig, ResourceLimitsError,
};
pub use occurrence::{ContentOccurrence, ContentRole, OccurrenceContext};
pub use request::{
    HttpStatusCode, HttpStatusCodeError, RequestMetadata, RequestMethod, RequestRoute,
    ResponseMetadata,
};
pub use response_stream::{ResponseLimitReason, StreamDecision, StreamingResponseBudget};
pub use structural_budget::{
    JsonBudgetAxis, JsonInspectionDecision, JsonShape, ProcessingBudget, ProcessingBudgetAxis,
    ProcessingBudgetDecision, ProcessingCharge, QueueDecision, classify_ipc_queue,
    classify_json_shape,
};
