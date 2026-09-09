//! Typed `UUIDv7` rejection contract tests.

use std::str::FromStr as _;

use tracepress_core::{IdParseError, SessionId};

#[test]
fn uuid_id_parse_rejects_non_v7_with_typed_error() {
    // Given
    let version_four = "550e8400-e29b-41d4-a716-446655440000";

    // When
    let result = SessionId::from_str(version_four);

    // Then
    assert!(matches!(
        result,
        Err(IdParseError::UnexpectedVersion { actual: 4, .. })
    ));
}

#[test]
fn uuid_id_deserialization_rejects_non_v7() {
    // Given
    let version_four_json = "\"550e8400-e29b-41d4-a716-446655440000\"";

    // When
    let result = serde_json::from_str::<SessionId>(version_four_json);

    // Then
    assert!(result.is_err());
}
