//! Identity contract of the analysis ids the daemon allocates.

use std::str::FromStr as _;

use tracepress_core::{ContextBlockOccurrenceId, ContextSnapshotId, IdParseError, UuidV7Generator};

#[test]
fn an_allocated_snapshot_id_survives_display_and_parse() -> Result<(), IdParseError> {
    // Given
    let generator = UuidV7Generator::new();
    let snapshot_id = ContextSnapshotId::generate(&generator);
    let block_id = ContextBlockOccurrenceId::generate(&generator);

    // When
    let parsed_snapshot = ContextSnapshotId::from_str(&snapshot_id.to_string())?;
    let parsed_block = ContextBlockOccurrenceId::from_str(&block_id.to_string())?;

    // Then
    assert_eq!(parsed_snapshot, snapshot_id);
    assert_eq!(parsed_block, block_id);
    Ok(())
}

#[test]
fn allocated_ids_are_distinct_within_one_generator() {
    // Given
    let generator = UuidV7Generator::new();

    // When
    let first = ContextSnapshotId::generate(&generator);
    let second = ContextSnapshotId::generate(&generator);

    // Then
    assert_ne!(first, second);
}

#[test]
fn a_non_v7_uuid_is_rejected_as_a_snapshot_id() {
    // Given
    let version_four = "550e8400-e29b-41d4-a716-446655440000";

    // When
    let result = ContextSnapshotId::from_str(version_four);

    // Then
    assert!(matches!(
        result,
        Err(IdParseError::UnexpectedVersion { actual: 4, .. })
    ));
}

#[test]
fn a_malformed_string_is_rejected_as_a_block_occurrence_id() {
    // Given
    let malformed = "not-a-uuid";

    // When
    let result = ContextBlockOccurrenceId::from_str(malformed);

    // Then
    assert!(matches!(result, Err(IdParseError::InvalidUuid(_))));
}
