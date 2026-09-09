use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;

use crate::OperationId;

/// A typed semantic relationship from a parent operation to a child operation.
#[allow(
    clippy::exhaustive_enums,
    reason = "causal semantics must be explicitly selected rather than stored as arbitrary strings"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalRelationship {
    /// The parent created the child operation.
    Spawned,
    /// The child continues work causally after the parent.
    FollowsFrom,
    /// The child recovers information associated with the parent.
    RecoveryOf,
    /// The child evaluates the parent's outcome.
    EvaluationOf,
}

/// A validated directed edge in the operation DAG.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CausalEdge {
    parent_operation_id: OperationId,
    child_operation_id: OperationId,
    relationship: CausalRelationship,
}

impl CausalEdge {
    /// Creates a directed causal edge between distinct operations.
    ///
    /// # Errors
    /// Returns [`CausalEdgeError`] when the parent and child are the same operation.
    pub fn new(
        parent_operation_id: OperationId,
        child_operation_id: OperationId,
        relationship: CausalRelationship,
    ) -> Result<Self, CausalEdgeError> {
        if parent_operation_id == child_operation_id {
            Err(CausalEdgeError::SelfEdge {
                operation_id: parent_operation_id,
            })
        } else {
            Ok(Self {
                parent_operation_id,
                child_operation_id,
                relationship,
            })
        }
    }

    /// Returns the parent operation identity.
    #[must_use]
    pub const fn parent_operation_id(&self) -> OperationId {
        self.parent_operation_id
    }

    /// Returns the child operation identity.
    #[must_use]
    pub const fn child_operation_id(&self) -> OperationId {
        self.child_operation_id
    }

    /// Returns the typed causal relationship.
    #[must_use]
    pub const fn relationship(&self) -> CausalRelationship {
        self.relationship
    }
}

impl<'de> Deserialize<'de> for CausalEdge {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let serialized = SerializedCausalEdge::deserialize(deserializer)?;
        Self::new(
            serialized.parent_operation_id,
            serialized.child_operation_id,
            serialized.relationship,
        )
        .map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
struct SerializedCausalEdge {
    parent_operation_id: OperationId,
    child_operation_id: OperationId,
    relationship: CausalRelationship,
}

/// Failure to construct a valid causal operation edge.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum CausalEdgeError {
    /// An operation cannot be its own causal parent.
    #[error("operation {operation_id} cannot have a causal edge to itself")]
    SelfEdge {
        /// The operation used as both endpoints.
        operation_id: OperationId,
    },
}
