//! Public ID and byte-content contract tests.

use std::str::FromStr as _;

use tracepress_core::{
    AttemptId, BindingId, ContentId, ContentKind, ContentObject, DecisionId, EvaluationId, EventId,
    IdParseError, OccurrenceId, OperationId, PolicyAssignmentId, RecoveryId, RequestId, SessionId,
    ToolCallId, UuidV7Generator, UuidV7Timestamp,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn generated_ids_are_uuid_v7_and_monotonic_when_timestamp_is_shared() -> TestResult {
    // Given
    let generator = UuidV7Generator::new();
    let timestamp = UuidV7Timestamp::new(1_704_067_200, 123_456_789)?;

    // When
    let first = SessionId::generate_at(&generator, timestamp);
    let second = SessionId::generate_at(&generator, timestamp);

    // Then
    assert_eq!(first.as_uuid().get_version_num(), 7);
    assert!(first < second);
    Ok(())
}

#[test]
fn every_external_identity_category_generates_uuid_v7() -> TestResult {
    // Given
    let generator = UuidV7Generator::new();
    let timestamp = UuidV7Timestamp::new(1_704_067_200, 0)?;

    // When
    let versions = [
        OperationId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        RequestId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        AttemptId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        ToolCallId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        DecisionId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        RecoveryId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        EvaluationId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        PolicyAssignmentId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        OccurrenceId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        BindingId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
        EventId::generate_at(&generator, timestamp)
            .as_uuid()
            .get_version_num(),
    ];

    // Then
    assert_eq!(versions, [7; 11]);
    Ok(())
}

#[test]
fn uuid_id_serializes_canonically_and_round_trips() -> TestResult {
    // Given
    let canonical = "01890f3e-7b19-7cc3-98c4-dc0c0c07398f";
    let session_id = SessionId::from_str(canonical)?;

    // When
    let json = serde_json::to_string(&session_id)?;
    let round_trip: SessionId = serde_json::from_str(&json)?;

    // Then
    assert_eq!(json, format!("\"{canonical}\""));
    assert_eq!(round_trip, session_id);
    Ok(())
}

#[test]
fn uuidv7_id_rejects_non_rfc_variant() {
    // Given
    let non_rfc_uuidv7 = "01890f3e-7b19-7000-0000-000000000001";

    // When
    let result = SessionId::from_str(non_rfc_uuidv7);

    // Then
    assert!(matches!(
        result,
        Err(IdParseError::UnexpectedVariant { .. })
    ));
}

#[test]
fn content_id_is_sha256_of_exact_raw_bytes() {
    // Given
    let raw = b"abc";

    // When
    let content_id = ContentId::from_bytes(raw);

    // Then
    assert_eq!(
        content_id.to_string(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn byte_distinctions_produce_distinct_content_ids() {
    // Given
    let payloads: [&[u8]; 5] = [b"line\n", b"line\r\n", "é".as_bytes(), &[0xe9], b"line\0"];

    // When
    let ids = payloads.map(ContentId::from_bytes);

    // Then
    for (left_index, left) in ids.iter().enumerate() {
        for right in ids.iter().skip(left_index.saturating_add(1)) {
            assert_ne!(left, right);
        }
    }
}

#[test]
fn content_round_trip_preserves_invalid_utf8_nul_and_binary() -> TestResult {
    // Given
    let raw = [0xff, 0xfe, 0x00, 0x7f, 0x89, b'P', b'N', b'G'];
    let content = ContentObject::new(&raw, ContentKind::Binary);

    // When
    let json = serde_json::to_vec(&content)?;
    let round_trip: ContentObject = serde_json::from_slice(&json)?;

    // Then
    assert_eq!(round_trip.raw_bytes(), raw);
    assert_eq!(round_trip.content_id(), ContentId::from_bytes(&raw));
    assert_eq!(round_trip.kind(), ContentKind::Binary);
    Ok(())
}

#[test]
fn content_kinds_serialize_to_the_complete_canonical_vocabulary() -> TestResult {
    // Given
    let kinds = [
        ContentKind::Text,
        ContentKind::Json,
        ContentKind::Ndjson,
        ContentKind::Log,
        ContentKind::SearchResults,
        ContentKind::TestResults,
        ContentKind::SourceCode,
        ContentKind::Diff,
        ContentKind::Image,
        ContentKind::Document,
        ContentKind::Binary,
        ContentKind::Unknown,
    ];

    // When
    let values: Result<Vec<String>, _> = kinds.iter().map(serde_json::to_string).collect();

    // Then
    assert_eq!(
        values?,
        [
            "\"text\"",
            "\"json\"",
            "\"ndjson\"",
            "\"log\"",
            "\"search_results\"",
            "\"test_results\"",
            "\"source_code\"",
            "\"diff\"",
            "\"image\"",
            "\"document\"",
            "\"binary\"",
            "\"unknown\"",
        ]
    );
    Ok(())
}
