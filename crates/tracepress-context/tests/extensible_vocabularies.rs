//! Forward tolerance of the provider-driven context vocabularies.

use tracepress_context::{
    ContextBlockKind, ContextOrigin, ContextRole, DetectedContentKind, DetectionConfidence,
    DetectionResult,
};

type TestResult = Result<(), serde_json::Error>;

#[test]
fn an_unrecognized_wire_value_deserializes_as_unknown() -> TestResult {
    // Given
    let future_item_kind = "\"web_search_call_v9\"";
    let future_role = "\"critic\"";
    let future_origin = "\"provider_synthesized\"";
    let future_content_kind = "\"protobuf\"";

    // When
    let kind = serde_json::from_str::<ContextBlockKind>(future_item_kind)?;
    let role = serde_json::from_str::<ContextRole>(future_role)?;
    let origin = serde_json::from_str::<ContextOrigin>(future_origin)?;
    let content_kind = serde_json::from_str::<DetectedContentKind>(future_content_kind)?;

    // Then
    assert_eq!(kind, ContextBlockKind::Unknown);
    assert_eq!(role, ContextRole::Unknown);
    assert_eq!(origin, ContextOrigin::Unknown);
    assert_eq!(content_kind, DetectedContentKind::Unknown);
    Ok(())
}

#[test]
fn a_block_record_carrying_an_unrecognized_kind_still_deserializes() -> TestResult {
    // Given
    let transported = r#"{"kind":"protobuf","confidence":"low","detector_version":1}"#;

    // When
    let result = serde_json::from_str::<DetectionResult>(transported)?;

    // Then
    assert_eq!(result.kind, DetectedContentKind::Unknown);
    assert_eq!(result.confidence, DetectionConfidence::Low);
    Ok(())
}

#[test]
fn the_unknown_member_round_trips_without_becoming_a_recognized_member() -> TestResult {
    // Given
    let unknown = ContextBlockKind::Unknown;

    // When
    let encoded = serde_json::to_string(&unknown)?;
    let decoded = serde_json::from_str::<ContextBlockKind>(&encoded)?;

    // Then
    assert_eq!(encoded, "\"unknown\"");
    assert_eq!(decoded, ContextBlockKind::Unknown);
    Ok(())
}

#[test]
fn recognized_members_keep_their_durable_wire_names() -> TestResult {
    // Given
    let kind = ContextBlockKind::ProviderStateReference;
    let origin = ContextOrigin::HumanAuthored;
    let content_kind = DetectedContentKind::SearchResults;

    // When
    let encoded_kind = serde_json::to_string(&kind)?;
    let encoded_origin = serde_json::to_string(&origin)?;
    let encoded_content_kind = serde_json::to_string(&content_kind)?;

    // Then
    assert_eq!(encoded_kind, "\"provider_state_reference\"");
    assert_eq!(encoded_origin, "\"human_authored\"");
    assert_eq!(encoded_content_kind, "\"search_results\"");
    assert_eq!(
        serde_json::from_str::<ContextBlockKind>(&encoded_kind)?,
        kind
    );
    assert_eq!(
        serde_json::from_str::<ContextOrigin>(&encoded_origin)?,
        origin
    );
    assert_eq!(
        serde_json::from_str::<DetectedContentKind>(&encoded_content_kind)?,
        content_kind
    );
    Ok(())
}
