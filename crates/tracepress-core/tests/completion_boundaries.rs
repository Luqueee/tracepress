//! Pure inference finalization contract tests.

use tracepress_core::{
    CommitState, FinalizationError, InferenceStatus, OperationStatus, SessionState,
    finalize_inference,
};

#[test]
fn complete_and_committed_inference_is_reported_completed() {
    // Given
    let session = SessionState::Active;

    // When
    let result = finalize_inference(session, InferenceStatus::Completed, CommitState::Committed);

    // Then
    assert_eq!(result, Ok(OperationStatus::Completed));
}

#[test]
fn misleading_success_is_rejected_until_commit_is_certain() {
    // Given
    let session = SessionState::Active;

    // When
    let result = finalize_inference(session, InferenceStatus::Completed, CommitState::Uncertain);

    // Then
    assert_eq!(
        result,
        Err(FinalizationError::SuccessNotCommitted {
            commit_state: CommitState::Uncertain,
        })
    );
}

#[test]
fn cancellation_never_becomes_success() {
    // Given
    let session = SessionState::Active;

    // When
    let result = finalize_inference(session, InferenceStatus::Cancelled, CommitState::Pending);

    // Then
    assert_eq!(result, Ok(OperationStatus::Cancelled));
}

#[test]
fn incomplete_response_never_becomes_success() {
    // Given
    let session = SessionState::Active;

    // When
    let result = finalize_inference(session, InferenceStatus::Incomplete, CommitState::Pending);

    // Then
    assert_eq!(result, Ok(OperationStatus::Incomplete));
}

#[test]
fn disconnected_response_never_becomes_success() {
    // Given
    let session = SessionState::Active;

    // When
    let result = finalize_inference(session, InferenceStatus::Disconnected, CommitState::Pending);

    // Then
    assert_eq!(result, Ok(OperationStatus::Disconnected));
}

#[test]
fn stale_session_rejects_even_claimed_completed_state() {
    // Given
    let session = SessionState::Stale;

    // When
    let result = finalize_inference(session, InferenceStatus::Completed, CommitState::Committed);

    // Then
    assert_eq!(result, Err(FinalizationError::StaleSession));
}

#[test]
fn every_session_inference_and_commit_combination_matches_the_independent_model() {
    // Given
    let sessions = [
        SessionState::Active,
        SessionState::Closing,
        SessionState::Closed,
        SessionState::Stale,
    ];
    let inferences = [
        InferenceStatus::Started,
        InferenceStatus::Streaming,
        InferenceStatus::Completed,
        InferenceStatus::Incomplete,
        InferenceStatus::Cancelled,
        InferenceStatus::Errored,
        InferenceStatus::Disconnected,
    ];
    let commits = [
        CommitState::Committed,
        CommitState::Pending,
        CommitState::Uncertain,
    ];
    let mut combinations = 0_u8;

    for session in sessions {
        for inference in inferences {
            for commit in commits {
                let expected = expected_finalization(session, inference, commit);

                // When
                let actual = finalize_inference(session, inference, commit);

                // Then
                assert_eq!(actual, expected);
                combinations += 1;
            }
        }
    }
    assert_eq!(combinations, 84);
}

const fn expected_finalization(
    session: SessionState,
    inference: InferenceStatus,
    commit: CommitState,
) -> Result<OperationStatus, FinalizationError> {
    match session {
        SessionState::Closed => Err(FinalizationError::ClosedSession),
        SessionState::Stale => Err(FinalizationError::StaleSession),
        SessionState::Active | SessionState::Closing => match inference {
            InferenceStatus::Started | InferenceStatus::Streaming => {
                Err(FinalizationError::InferenceStillRunning { status: inference })
            }
            InferenceStatus::Completed => match commit {
                CommitState::Committed => Ok(OperationStatus::Completed),
                CommitState::Pending | CommitState::Uncertain => {
                    Err(FinalizationError::SuccessNotCommitted {
                        commit_state: commit,
                    })
                }
            },
            InferenceStatus::Incomplete => Ok(OperationStatus::Incomplete),
            InferenceStatus::Cancelled => Ok(OperationStatus::Cancelled),
            InferenceStatus::Errored => Ok(OperationStatus::Errored),
            InferenceStatus::Disconnected => Ok(OperationStatus::Disconnected),
        },
    }
}
