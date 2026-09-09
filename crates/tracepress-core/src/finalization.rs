use thiserror::Error;

use crate::{InferenceStatus, OperationStatus, SessionState};

/// Persistence certainty accompanying a terminal forwarding state.
#[allow(
    clippy::exhaustive_enums,
    reason = "success reporting must distinguish committed, pending, and uncertain persistence"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitState {
    /// The critical operation was committed.
    Committed,
    /// The critical operation has not committed yet.
    Pending,
    /// Interruption left commit status uncertain.
    Uncertain,
}

/// Failure to finalize an inference without inventing success.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum FinalizationError {
    /// A stale session cannot accept state transitions.
    #[error("stale session cannot finalize an inference")]
    StaleSession,
    /// A closed session cannot accept state transitions.
    #[error("closed session cannot finalize an inference")]
    ClosedSession,
    /// A non-terminal inference state cannot be finalized.
    #[error("inference is still in non-terminal state {status:?}")]
    InferenceStillRunning {
        /// The rejected non-terminal status.
        status: InferenceStatus,
    },
    /// Claimed success lacked a certain critical commit.
    #[error("completed inference cannot report success while commit is {commit_state:?}")]
    SuccessNotCommitted {
        /// The persistence state that prevented success.
        commit_state: CommitState,
    },
}

/// Resolves a terminal inference into an operation status at the pure domain transition seam.
///
/// # Errors
/// Returns [`FinalizationError`] for stale or closed sessions, non-terminal inference states, or
/// success without a certain critical commit.
pub const fn finalize_inference(
    session_state: SessionState,
    inference_status: InferenceStatus,
    commit_state: CommitState,
) -> Result<OperationStatus, FinalizationError> {
    match session_state {
        SessionState::Active | SessionState::Closing => {}
        SessionState::Closed => return Err(FinalizationError::ClosedSession),
        SessionState::Stale => return Err(FinalizationError::StaleSession),
    }
    match inference_status {
        InferenceStatus::Started | InferenceStatus::Streaming => {
            Err(FinalizationError::InferenceStillRunning {
                status: inference_status,
            })
        }
        InferenceStatus::Completed => match commit_state {
            CommitState::Committed => Ok(OperationStatus::Completed),
            CommitState::Pending | CommitState::Uncertain => {
                Err(FinalizationError::SuccessNotCommitted { commit_state })
            }
        },
        InferenceStatus::Incomplete => Ok(OperationStatus::Incomplete),
        InferenceStatus::Cancelled => Ok(OperationStatus::Cancelled),
        InferenceStatus::Errored => Ok(OperationStatus::Errored),
        InferenceStatus::Disconnected => Ok(OperationStatus::Disconnected),
    }
}
