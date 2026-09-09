//! Representative enum serialization round-trip tests.

use serde::{Serialize, de::DeserializeOwned};
use tracepress_core::{ContentKind, InferenceStatus, OperationKind, SessionState, UsageStatus};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn representative_content_enum_round_trips() -> TestResult {
    // Given
    let content_kind = ContentKind::Unknown;

    // When
    let round_trip = json_round_trip(content_kind)?;

    // Then
    assert_eq!(round_trip, content_kind);
    Ok(())
}

#[test]
fn representative_lifecycle_enums_round_trip() -> TestResult {
    // Given
    let operation_kind = OperationKind::ProviderTool;
    let session_state = SessionState::Stale;
    let inference_status = InferenceStatus::Disconnected;
    let usage_status = UsageStatus::Unavailable;

    // When
    let operation_round_trip = json_round_trip(operation_kind)?;
    let session_round_trip = json_round_trip(session_state)?;
    let inference_round_trip = json_round_trip(inference_status)?;
    let usage_round_trip = json_round_trip(usage_status)?;

    // Then
    assert_eq!(operation_round_trip, operation_kind);
    assert_eq!(session_round_trip, session_state);
    assert_eq!(inference_round_trip, inference_status);
    assert_eq!(usage_round_trip, usage_status);
    Ok(())
}

fn json_round_trip<T>(value: T) -> Result<T, serde_json::Error>
where
    T: Copy + DeserializeOwned + Serialize,
{
    serde_json::from_str(&serde_json::to_string(&value)?)
}
