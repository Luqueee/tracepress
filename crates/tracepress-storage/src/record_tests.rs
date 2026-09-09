use rusqlite::{Connection, ErrorCode};
use tempfile::TempDir;
use tracepress_core::{
    BindingCandidate, BindingId, BindingKey, BindingRepresentation, BindingVersions, CausalEdge,
    CausalRelationship, ContentKind, ContentObject, ContentOccurrence, ContentRole, DecisionId,
    EventId, MaxIpcQueueItems, OccurrenceContext, OccurrenceId, OperationId, OperationKind,
    OperationStatus, PolicyAssignmentId, RecoveryId, SessionId, SessionState, UuidV7Generator,
};

use crate::test_support::TestResult;
use crate::{
    Durability, FidelityClass, StorageConfig, StorageWriter, WriteBatch, WriteCommand, WriteReceipt,
};

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one canonical fixture demonstrates the complete foreign-key-valid record graph"
)]
async fn canonical_records_commit_with_frozen_xor_and_append_only_constraints() -> TestResult {
    // Given: typed domain records forming one complete foreign-key-valid logical set.
    let directory = TempDir::new()?;
    let database = directory.path().join("canonical.sqlite3");
    let generator = UuidV7Generator::new();
    let session_id = SessionId::generate(&generator);
    let parent_id = OperationId::generate(&generator);
    let child_id = OperationId::generate(&generator);
    let decision_id = DecisionId::generate(&generator);
    let raw = ContentObject::new(b"raw", ContentKind::Text);
    let rendered = ContentObject::new(b"rendered", ContentKind::Text);
    let occurrence = ContentOccurrence::new(
        OccurrenceId::generate(&generator),
        raw.content_id(),
        OccurrenceContext::new(session_id, Some(child_id), ContentRole::TracepressResult),
    );
    let binding = tracepress_core::ContextBinding::freeze(
        BindingId::generate(&generator),
        BindingCandidate::new(
            BindingKey::new(session_id, raw.content_id()),
            BindingRepresentation::new(
                raw.content_id(),
                rendered.content_id(),
                BindingVersions::new("identity-v1", "baseline-v1"),
            ),
        ),
    );
    let queue_items = MaxIpcQueueItems::new(4)?;
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Strict,
        queue_items,
    ))
    .await?;

    // When: every remaining canonical record type commits in one ordered batch.
    let receipt = writer
        .submit_batch(
            WriteBatch::new(session(session_id))
                .and(operation(parent_id, session_id, OperationKind::Agent))
                .and(operation(child_id, session_id, OperationKind::Compression))
                .and(WriteCommand::CausalEdge {
                    edge: CausalEdge::new(parent_id, child_id, CausalRelationship::Spawned)?,
                })
                .and(WriteCommand::ContentObject {
                    object: raw.clone(),
                    pin_count: 1,
                    created_at: "2026-09-08T21:00:02Z".to_owned(),
                })
                .and(WriteCommand::ContentObject {
                    object: rendered.clone(),
                    pin_count: 0,
                    created_at: "2026-09-08T21:00:03Z".to_owned(),
                })
                .and(WriteCommand::ContentOccurrence {
                    occurrence,
                    observed_at: "2026-09-08T21:00:04Z".to_owned(),
                })
                .and(WriteCommand::ContentBinding { binding })
                .and(WriteCommand::CompressionDecision {
                    decision_id,
                    session_id,
                    operation_id: child_id,
                    input_content_id: raw.content_id(),
                    output_content_id: rendered.content_id(),
                    fidelity: FidelityClass::Exact,
                    recoverable: true,
                    compressor: "identity".to_owned(),
                    compressor_version: "identity-v1".to_owned(),
                    policy_version: "baseline-v1".to_owned(),
                    raw_bytes: 3,
                    output_bytes: 8,
                    estimated_raw_tokens: None,
                    estimated_output_tokens: None,
                    target_tokens: None,
                    latency_us: None,
                    feature_schema_version: "1".to_owned(),
                    features: None,
                })
                .and(WriteCommand::Recovery {
                    recovery_id: RecoveryId::generate(&generator),
                    decision_id,
                    raw_content_id: raw.content_id(),
                    provenance_content_id: None,
                })
                .and(WriteCommand::PolicyAssignment {
                    policy_assignment_id: PolicyAssignmentId::generate(&generator),
                    session_id,
                    policy_version: "baseline-v1".to_owned(),
                    assigned_at: "2026-09-08T21:00:05Z".to_owned(),
                    chosen_action: "identity".to_owned(),
                    candidate_actions: None,
                    action_probability: None,
                    random_seed: None,
                    feature_vector: None,
                })
                .and(WriteCommand::Event {
                    event_id: EventId::generate(&generator),
                    session_id: Some(session_id),
                    operation_id: Some(child_id),
                    timestamp: "2026-09-08T21:00:06Z".to_owned(),
                    event_type: "compression.persisted".to_owned(),
                    payload: b"{}".as_slice().into(),
                    schema_version: "1".to_owned(),
                }),
        )
        .await?;
    writer.shutdown().await?;

    // Then: all rows exist and direct violations of immutable constraints fail.
    assert!(matches!(
        receipt,
        WriteReceipt::BatchCommitted { rows_changed: 12 }
    ));
    let connection = Connection::open(database)?;
    assert_constraint(connection.execute("UPDATE events SET payload = X'00'", []));
    assert_constraint(connection.execute("DELETE FROM events", []));
    assert_constraint(connection.execute(
        "INSERT INTO content_objects(content_id, raw_bytes, external_ref, byte_length, content_kind, created_at) VALUES ('invalid', NULL, NULL, 0, 'text', 'now')",
        [],
    ));
    assert_constraint(connection.execute("UPDATE content_bindings SET frozen = 0", []));
    Ok(())
}

fn session(session_id: SessionId) -> WriteCommand {
    WriteCommand::Session {
        session_id,
        started_at: "2026-09-08T21:00:00Z".to_owned(),
        ended_at: None,
        state: SessionState::Active,
        ingress_key: "canonical-ingress".to_owned(),
    }
}

fn operation(
    operation_id: OperationId,
    session_id: SessionId,
    kind: OperationKind,
) -> WriteCommand {
    WriteCommand::Operation {
        operation_id,
        session_id,
        kind,
        started_at: "2026-09-08T21:00:01Z".to_owned(),
        ended_at: None,
        status: OperationStatus::Started,
    }
}

fn assert_constraint(result: rusqlite::Result<usize>) {
    assert!(matches!(
        result,
        Err(error) if error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation)
    ));
}
