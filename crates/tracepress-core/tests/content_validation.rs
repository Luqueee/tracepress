//! Content identity parsing and integrity contract tests.

use std::str::FromStr as _;

use tracepress_core::{
    ContentId, ContentIdParseError, ContentKind, ContentObject, ContentObjectError, RawContent,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn content_id_parse_accepts_canonical_sha256_and_round_trips() -> TestResult {
    // Given
    let canonical = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    // When
    let parsed = ContentId::from_str(canonical)?;

    // Then
    assert_eq!(parsed, ContentId::from_bytes(b"abc"));
    assert_eq!(parsed.to_string(), canonical);
    Ok(())
}

#[test]
fn content_id_parse_rejects_invalid_length_with_typed_error() {
    // Given
    let short_digest = "ba78";

    // When
    let result = ContentId::from_str(short_digest);

    // Then
    assert!(matches!(
        result,
        Err(ContentIdParseError::InvalidLength { actual: 4, .. })
    ));
}

#[test]
fn content_id_parse_rejects_invalid_hex_with_typed_error() {
    // Given
    let invalid_hex = "za7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    // When
    let result = ContentId::from_str(invalid_hex);

    // Then
    assert!(matches!(
        result,
        Err(ContentIdParseError::InvalidHex { byte: b'z', .. })
    ));
}

#[test]
fn content_object_rejects_mismatched_declared_identity() {
    // Given
    let declared = ContentId::from_bytes(b"declared bytes");
    let raw = RawContent::new(b"actual bytes");
    let computed = ContentId::from_bytes(raw.as_bytes());

    // When
    let result = ContentObject::from_parts(declared, raw, ContentKind::Unknown);

    // Then
    assert_eq!(
        result,
        Err(ContentObjectError::ContentIdMismatch { declared, computed })
    );
}

#[test]
fn content_object_deserialization_rejects_mismatched_declared_identity() {
    // Given
    let value = serde_json::json!({
        "content_id": ContentId::from_bytes(b"declared bytes"),
        "raw_bytes": [97, 99, 116, 117, 97, 108, 32, 98, 121, 116, 101, 115],
        "content_kind": "binary"
    });

    // When
    let result = serde_json::from_value::<ContentObject>(value);

    // Then
    assert!(result.is_err());
}
