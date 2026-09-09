//! Causal lifecycle and request metadata contract tests.

use std::str::FromStr as _;

use serde::Serialize;
use tracepress_core::{
    CausalEdge, CausalEdgeError, CausalRelationship, InferenceStatus, OperationId, OperationKind,
    OperationStatus, RequestId, RequestMetadata, RequestMethod, RequestRoute, SessionState,
    UsageStatus,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn operation_and_lifecycle_enums_use_canonical_typed_values() -> TestResult {
    // Given
    let operation_kinds = [
        OperationKind::Agent,
        OperationKind::LlmInference,
        OperationKind::ToolExecution,
        OperationKind::Compression,
        OperationKind::Recovery,
        OperationKind::Evaluation,
        OperationKind::ProviderTool,
    ];
    let inference_statuses = [
        InferenceStatus::Started,
        InferenceStatus::Streaming,
        InferenceStatus::Completed,
        InferenceStatus::Incomplete,
        InferenceStatus::Cancelled,
        InferenceStatus::Errored,
        InferenceStatus::Disconnected,
    ];
    let usage_statuses = [
        UsageStatus::Final,
        UsageStatus::Partial,
        UsageStatus::Unavailable,
    ];
    let session_states = [
        SessionState::Active,
        SessionState::Closing,
        SessionState::Closed,
        SessionState::Stale,
    ];
    let operation_statuses = [
        OperationStatus::Started,
        OperationStatus::Completed,
        OperationStatus::Incomplete,
        OperationStatus::Cancelled,
        OperationStatus::Errored,
        OperationStatus::Disconnected,
    ];

    // When
    let operation_json = serialize_each(&operation_kinds)?;
    let inference_json = serialize_each(&inference_statuses)?;
    let usage_json = serialize_each(&usage_statuses)?;
    let session_json = serialize_each(&session_states)?;
    let operation_status_json = serialize_each(&operation_statuses)?;

    // Then
    assert_eq!(
        operation_json,
        [
            "\"agent\"",
            "\"llm_inference\"",
            "\"tool_execution\"",
            "\"compression\"",
            "\"recovery\"",
            "\"evaluation\"",
            "\"provider_tool\"",
        ]
    );
    assert_eq!(
        inference_json,
        [
            "\"started\"",
            "\"streaming\"",
            "\"completed\"",
            "\"incomplete\"",
            "\"cancelled\"",
            "\"errored\"",
            "\"disconnected\"",
        ]
    );
    assert_eq!(usage_json, ["\"final\"", "\"partial\"", "\"unavailable\""]);
    assert_eq!(
        session_json,
        ["\"active\"", "\"closing\"", "\"closed\"", "\"stale\""]
    );
    assert_eq!(
        operation_status_json,
        [
            "\"started\"",
            "\"completed\"",
            "\"incomplete\"",
            "\"cancelled\"",
            "\"errored\"",
            "\"disconnected\"",
        ]
    );
    Ok(())
}

#[test]
fn causal_edge_rejects_a_self_edge_with_typed_error() -> TestResult {
    // Given
    let operation_id = OperationId::from_str("01890f3e-7b19-7000-8000-000000000010")?;

    // When
    let result = CausalEdge::new(operation_id, operation_id, CausalRelationship::FollowsFrom);

    // Then
    assert_eq!(result, Err(CausalEdgeError::SelfEdge { operation_id }));
    Ok(())
}

#[test]
fn causal_edge_round_trips_typed_relationship() -> TestResult {
    // Given
    let parent = OperationId::from_str("01890f3e-7b19-7000-8000-000000000010")?;
    let child = OperationId::from_str("01890f3e-7b19-7000-8000-000000000011")?;
    let edge = CausalEdge::new(parent, child, CausalRelationship::Spawned)?;

    // When
    let json = serde_json::to_vec(&edge)?;
    let round_trip: CausalEdge = serde_json::from_slice(&json)?;

    // Then
    assert_eq!(round_trip, edge);
    assert_eq!(round_trip.parent_operation_id(), parent);
    assert_eq!(round_trip.child_operation_id(), child);
    assert_eq!(round_trip.relationship(), CausalRelationship::Spawned);
    Ok(())
}

#[test]
fn unknown_request_metadata_is_null_and_never_zero_filled() -> TestResult {
    // Given
    let request_id = RequestId::from_str("01890f3e-7b19-7000-8000-000000000012")?;
    let metadata = RequestMetadata::chat_completions(request_id, 37);

    // When
    let value = serde_json::to_value(metadata)?;

    // Then
    assert_eq!(metadata.request_id(), request_id);
    assert_eq!(metadata.route(), RequestRoute::ChatCompletions);
    assert_eq!(metadata.method(), RequestMethod::Post);
    assert_eq!(metadata.request_bytes(), 37);
    assert!(
        value
            .get("status_code")
            .is_some_and(serde_json::Value::is_null)
    );
    assert!(
        value
            .get("response_bytes")
            .is_some_and(serde_json::Value::is_null)
    );
    assert!(
        value
            .get("latency_us")
            .is_some_and(serde_json::Value::is_null)
    );
    Ok(())
}

#[test]
fn absent_and_explicit_null_request_metadata_both_remain_unknown() -> TestResult {
    // Given
    let absent = serde_json::json!({
        "request_id": "01890f3e-7b19-7000-8000-000000000012",
        "route": "chat_completions",
        "method": "post",
        "request_bytes": 37
    });
    let explicit_null = serde_json::json!({
        "request_id": "01890f3e-7b19-7000-8000-000000000012",
        "route": "chat_completions",
        "method": "post",
        "request_bytes": 37,
        "status_code": null,
        "response_bytes": null,
        "latency_us": null
    });

    // When
    let from_absent: RequestMetadata = serde_json::from_value(absent)?;
    let from_null: RequestMetadata = serde_json::from_value(explicit_null)?;

    // Then
    assert_eq!(from_absent.status_code(), None);
    assert_eq!(from_absent.response_bytes(), None);
    assert_eq!(from_absent.latency_us(), None);
    assert_eq!(from_absent, from_null);
    Ok(())
}

fn serialize_each<T: Serialize>(values: &[T]) -> Result<Vec<String>, serde_json::Error> {
    values.iter().map(serde_json::to_string).collect()
}
