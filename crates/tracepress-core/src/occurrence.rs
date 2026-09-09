use serde::{Deserialize, Serialize};

use crate::{ContentId, OccurrenceId, OperationId, SessionId};

/// The pipeline location at which exact content bytes were observed.
#[allow(
    clippy::exhaustive_enums,
    reason = "the canonical schema requires consumers to distinguish every observation level"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentRole {
    /// Bytes emitted by the executed process.
    ProcessOutput,
    /// Bytes exposed to the agent as a tool result.
    AgentToolResult,
    /// Bytes selected by Tracepress for a provider request.
    TracepressResult,
    /// Bytes observed in the final provider input.
    ProviderInput,
}

/// Session and operation context for a content occurrence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OccurrenceContext {
    session_id: SessionId,
    operation_id: Option<OperationId>,
    role: ContentRole,
}

impl OccurrenceContext {
    /// Creates an occurrence context, preserving an unknown operation as `None`.
    #[must_use]
    pub const fn new(
        session_id: SessionId,
        operation_id: Option<OperationId>,
        role: ContentRole,
    ) -> Self {
        Self {
            session_id,
            operation_id,
            role,
        }
    }
}

/// One immutable observation of a content object in a session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentOccurrence {
    occurrence_id: OccurrenceId,
    content_id: ContentId,
    #[serde(flatten)]
    context: OccurrenceContext,
}

impl ContentOccurrence {
    /// Creates a typed content occurrence.
    #[must_use]
    pub const fn new(
        occurrence_id: OccurrenceId,
        content_id: ContentId,
        context: OccurrenceContext,
    ) -> Self {
        Self {
            occurrence_id,
            content_id,
            context,
        }
    }

    /// Returns the occurrence identity.
    #[must_use]
    pub const fn occurrence_id(&self) -> OccurrenceId {
        self.occurrence_id
    }

    /// Returns the observed content identity.
    #[must_use]
    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }

    /// Returns the containing session identity.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.context.session_id
    }

    /// Returns the causal operation when one is known.
    #[must_use]
    pub const fn operation_id(&self) -> Option<OperationId> {
        self.context.operation_id
    }

    /// Returns the pipeline observation level.
    #[must_use]
    pub const fn role(&self) -> ContentRole {
        self.context.role
    }
}
