//! Bounded metadata truncation and privacy contract tests.

use tracepress_context::{BoundedMetadataText, ContextDigest};

#[test]
fn a_short_semantic_path_is_kept_whole_without_a_digest() {
    // Given
    let path = "/input/3/content/0/text";

    // When
    let bounded = BoundedMetadataText::semantic_path(path);

    // Then
    assert_eq!(bounded.as_str(), path);
    assert!(!bounded.is_truncated());
    assert_eq!(bounded.full_value_hash(), None);
}

#[test]
fn an_over_long_semantic_path_is_truncated_and_keeps_a_digest_of_the_original() {
    // Given
    let path = format!("/input/{}/text", "a".repeat(4096));

    // When
    let bounded = BoundedMetadataText::semantic_path(&path);

    // Then
    assert!(bounded.is_truncated());
    assert!(bounded.as_str().len() <= BoundedMetadataText::SEMANTIC_PATH_MAX_BYTES);
    assert_eq!(bounded.original_bytes(), path.len() as u64);
    assert_eq!(
        bounded.full_value_hash(),
        Some(ContextDigest::from_bytes(path.as_bytes()))
    );
}

#[test]
fn two_equal_originals_truncate_to_the_same_digest() {
    // Given
    let first = "x".repeat(1_000);
    let second = "x".repeat(1_000);
    let different = "y".repeat(1_000);

    // When
    let first_hash = BoundedMetadataText::tool_name(&first).full_value_hash();
    let second_hash = BoundedMetadataText::tool_name(&second).full_value_hash();
    let different_hash = BoundedMetadataText::tool_name(&different).full_value_hash();

    // Then
    assert_eq!(first_hash, second_hash);
    assert_ne!(first_hash, different_hash);
}

#[test]
fn truncation_never_splits_a_multi_byte_character() {
    // Given
    let name = "😀".repeat(1_000);

    // When
    let bounded = BoundedMetadataText::tool_name(&name);

    // Then
    assert!(bounded.is_truncated());
    assert!(bounded.as_str().len() <= BoundedMetadataText::TOOL_NAME_MAX_BYTES);
    assert!(name.starts_with(bounded.as_str()));
    assert_eq!(bounded.as_str().chars().count(), bounded.as_str().len() / 4);
}

#[test]
fn the_debug_representation_omits_the_metadata_value() {
    // Given
    let path = "/input/0/content/0/api_key_of_the_customer";

    // When
    let rendered = format!("{:?}", BoundedMetadataText::semantic_path(path));

    // Then
    assert!(!rendered.contains("api_key_of_the_customer"), "{rendered}");
    assert!(rendered.contains("retained_bytes"), "{rendered}");
}

#[test]
fn a_transported_value_above_the_bound_is_rejected() {
    // Given
    let oversized = format!(
        r#"{{"value":"{}","original_bytes":100000,"full_value_hash":null}}"#,
        "a".repeat(BoundedMetadataText::MAX_BYTES + 1)
    );

    // When
    let result = serde_json::from_str::<BoundedMetadataText>(&oversized);

    // Then
    assert!(result.is_err());
}

#[test]
fn a_transported_truncation_without_its_digest_is_rejected() {
    // Given
    let inconsistent = r#"{"value":"/input","original_bytes":4096,"full_value_hash":null}"#;

    // When
    let result = serde_json::from_str::<BoundedMetadataText>(inconsistent);

    // Then
    assert!(result.is_err());
}

#[test]
fn a_bounded_value_round_trips_over_the_wire() -> Result<(), serde_json::Error> {
    // Given
    let bounded = BoundedMetadataText::semantic_path(&format!("/tools/{}", "n".repeat(1_000)));

    // When
    let decoded = serde_json::from_str::<BoundedMetadataText>(&serde_json::to_string(&bounded)?)?;

    // Then
    assert_eq!(decoded, bounded);
    assert!(decoded.is_truncated());
    Ok(())
}
