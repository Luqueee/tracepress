//! Content occurrence and frozen binding contract tests.

use std::str::FromStr as _;

use tracepress_core::{
    BindingCandidate, BindingConflictField, BindingId, BindingKey, BindingRepresentation,
    BindingVersions, ContentId, ContentOccurrence, ContentRole, ContextBinding,
    ContextBindingError, OccurrenceContext, OccurrenceId, OperationId, SessionId,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn content_occurrence_round_trips_typed_role_and_optional_operation() -> TestResult {
    // Given
    let occurrence_id = OccurrenceId::from_str("01890f3e-7b19-7000-8000-000000000001")?;
    let content_id = ContentId::from_bytes(b"tool result");
    let session_id = SessionId::from_str("01890f3e-7b19-7000-8000-000000000002")?;
    let operation_id = OperationId::from_str("01890f3e-7b19-7000-8000-000000000003")?;
    let occurrence = ContentOccurrence::new(
        occurrence_id,
        content_id,
        OccurrenceContext::new(session_id, Some(operation_id), ContentRole::AgentToolResult),
    );

    // When
    let json = serde_json::to_vec(&occurrence)?;
    let round_trip: ContentOccurrence = serde_json::from_slice(&json)?;

    // Then
    assert_eq!(round_trip, occurrence);
    assert_eq!(round_trip.role(), ContentRole::AgentToolResult);
    assert_eq!(round_trip.operation_id(), Some(operation_id));
    Ok(())
}

#[test]
fn repeated_identical_frozen_binding_is_idempotent() -> TestResult {
    // Given
    let binding_id = BindingId::from_str("01890f3e-7b19-7000-8000-000000000004")?;
    let candidate = binding_candidate(ContentId::from_bytes(b"rendered"))?;
    let binding = ContextBinding::freeze(binding_id, candidate.clone());

    // When
    let result = binding.ensure_compatible(&candidate);

    // Then
    assert_eq!(result, Ok(()));
    assert_eq!(binding.binding_id(), binding_id);
    assert_eq!(binding.session_id(), candidate.key().session_id());
    assert_eq!(
        binding.logical_content_id(),
        candidate.key().logical_content_id()
    );
    assert_eq!(
        binding.raw_content_id(),
        candidate.representation().raw_content_id()
    );
    assert_eq!(
        binding.rendered_content_id(),
        candidate.representation().rendered_content_id()
    );
    assert_eq!(binding.compressor_version(), "compressor-v1");
    assert_eq!(binding.policy_version(), "policy-v1");
    Ok(())
}

#[test]
fn rejects_conflicting_frozen_binding() -> TestResult {
    // Given
    let binding_id = BindingId::from_str("01890f3e-7b19-7000-8000-000000000004")?;
    let binding = ContextBinding::freeze(
        binding_id,
        binding_candidate(ContentId::from_bytes(b"first rendering"))?,
    );
    let conflicting = binding_candidate(ContentId::from_bytes(b"replacement rendering"))?;

    // When
    let result = binding.ensure_compatible(&conflicting);

    // Then
    assert_eq!(
        result,
        Err(ContextBindingError::FrozenConflict {
            binding_id,
            field: BindingConflictField::RenderedContent,
        })
    );
    Ok(())
}

#[test]
fn serialized_binding_is_always_frozen_and_rejects_false_marker() -> TestResult {
    // Given
    let binding_id = BindingId::from_str("01890f3e-7b19-7000-8000-000000000004")?;
    let binding = ContextBinding::freeze(
        binding_id,
        binding_candidate(ContentId::from_bytes(b"rendered"))?,
    );

    // When
    let mut value = serde_json::to_value(binding)?;

    // Then
    assert_eq!(value.get("frozen"), Some(&serde_json::Value::Bool(true)));
    if let Some(frozen) = value.get_mut("frozen") {
        *frozen = serde_json::Value::Bool(false);
    }
    assert!(serde_json::from_value::<ContextBinding>(value).is_err());
    Ok(())
}

fn binding_candidate(
    rendered_content_id: ContentId,
) -> Result<BindingCandidate, Box<dyn std::error::Error>> {
    let session_id = SessionId::from_str("01890f3e-7b19-7000-8000-000000000002")?;
    Ok(BindingCandidate::new(
        BindingKey::new(session_id, ContentId::from_bytes(b"logical")),
        BindingRepresentation::new(
            ContentId::from_bytes(b"raw"),
            rendered_content_id,
            BindingVersions::new("compressor-v1", "policy-v1"),
        ),
    ))
}
