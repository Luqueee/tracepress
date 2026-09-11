use rusqlite::Connection;
use tempfile::TempDir;
use tracepress_core::{
    AttemptId, EvaluationId, EventId, InferenceStatus, MaxIpcQueueItems, OperationId,
    OperationKind, OperationStatus, RequestId, RequestMetadata, SessionId, SessionState,
    UuidV7Generator,
};

use crate::test_support::TestResult;
use crate::{
    Durability, StorageConfig, StorageError, StorageWriter, WriteBatch, WriteCommand, WriteReceipt,
};

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one integration path demonstrates ordering and SQL NULL semantics across related records"
)]
async fn writer_serializes_events_and_persists_unknowns_as_null() -> TestResult {
    // Given: one migrated writer and typed IDs for a complete request path.
    let directory = TempDir::new()?;
    let database = directory.path().join("writer.sqlite3");
    let generator = UuidV7Generator::new();
    let session_id = SessionId::generate(&generator);
    let operation_id = OperationId::generate(&generator);
    let request_id = RequestId::generate(&generator);
    let attempt_id = AttemptId::generate(&generator);
    let evaluation_id = EvaluationId::generate(&generator);
    let queue_items = MaxIpcQueueItems::new(4)?;
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Balanced,
        queue_items,
    ))
    .await?;

    // When: typed commands record unknown usage, cost, and latency plus two events.
    committed(
        writer
            .submit(WriteCommand::Session {
                session_id,
                started_at: "2026-09-08T20:00:00Z".to_owned(),
                ended_at: None,
                state: SessionState::Active,
                ingress_key: "local-ingress".to_owned(),
            })
            .await?,
    );
    committed(
        writer
            .submit(WriteCommand::Operation {
                operation_id,
                session_id,
                kind: OperationKind::LlmInference,
                started_at: "2026-09-08T20:00:01Z".to_owned(),
                ended_at: None,
                status: OperationStatus::Started,
            })
            .await?,
    );
    committed(
        writer
            .submit(WriteCommand::ProviderRequest {
                operation_id,
                metadata: RequestMetadata::chat_completions(request_id, 19),
                provider: None,
                protocol: None,
                transport: None,
                endpoint_profile_version: None,
                content_encoding: None,
                analysis_decode_status: None,
                wire_bytes: None,
                wire_sha256: None,
                decoded_bytes: None,
                decode_duration_us: None,
                decoder_version: None,
                parser_version: None,
                observation_status: None,
                model: None,
                stream: None,
                background: None,
                store: None,
                reasoning_effort: None,
                text_verbosity: None,
                truncation: None,
                previous_response_id_present: None,
                input_item_count: None,
                tool_count: None,
                text_input_block_count: None,
                image_input_block_count: None,
                file_input_block_count: None,
            })
            .await?,
    );
    committed(
        writer
            .submit(WriteCommand::ProviderAttempt {
                attempt_id,
                request_id,
                ordinal: 0,
                status_code: None,
                started_at: "2026-09-08T20:00:02Z".to_owned(),
                ended_at: None,
                status: InferenceStatus::Started,
                provider_response_id: None,
                response_model: None,
                response_state: None,
                provider_created_at: None,
                incomplete_reason: None,
                error_code: None,
                transport_error: None,
                observation_status: None,
                streaming: None,
                chunk_count: None,
                byte_count: None,
                ttfb_us: None,
                ttft_us: None,
                duration_us: None,
                anomaly_metadata: None,
            })
            .await?,
    );
    committed(
        writer
            .submit(WriteCommand::ProviderUsage {
                attempt_id,
                input_total: None,
                input_uncached: None,
                cache_read: None,
                cache_write: None,
                output_total: None,
                reasoning: None,
                usage_status: None,
                raw_usage_json: None,
                input_cached: None,
                output_reasoning: None,
                total: None,
                normalizer_version: None,
                anomaly_metadata: None,
            })
            .await?,
    );
    committed(
        writer
            .submit(WriteCommand::Evaluation {
                evaluation_id,
                session_id,
                operation_id: Some(operation_id),
                provider_cost_microusd: None,
                input_tokens: None,
                cache_tokens: None,
                output_tokens: None,
                recoveries: None,
                reruns: None,
                latency_ms: None,
                task_success: None,
                user_correction: None,
                quality_score: None,
            })
            .await?,
    );
    let first = writer
        .submit(event(&generator, (session_id, operation_id), b"first"))
        .await?;
    let second = writer
        .submit(event(&generator, (session_id, operation_id), b"second"))
        .await?;
    writer.shutdown().await?;

    // Then: event receipts are monotonic and every unavailable metric is SQL NULL.
    assert_eq!(event_sequence(first), Some(1));
    assert_eq!(event_sequence(second), Some(2));
    let connection = Connection::open(database)?;
    let usage_nulls: i64 = connection
        .query_row(
            "SELECT (input_total IS NULL) + (input_uncached IS NULL) + (cache_read IS NULL) + (cache_write IS NULL) + (output_total IS NULL) + (reasoning IS NULL) + (usage_status IS NULL) FROM provider_usage",
            [],
            |row| row.get(0),
        )?;
    assert_eq!(usage_nulls, 7);
    let evaluation_nulls: i64 = connection
        .query_row(
            "SELECT (provider_cost_microusd IS NULL) + (input_tokens IS NULL) + (cache_tokens IS NULL) + (output_tokens IS NULL) + (latency_ms IS NULL) FROM evaluations",
            [],
            |row| row.get(0),
        )?;
    assert_eq!(evaluation_nulls, 5);
    Ok(())
}

#[tokio::test]
async fn interrupted_transaction_keeps_prior_logical_set() -> TestResult {
    // Given: a two-row foreign-key-valid batch for an empty real database.
    let directory = TempDir::new()?;
    let database = directory.path().join("transaction.sqlite3");
    let generator = UuidV7Generator::new();
    let session_id = SessionId::generate(&generator);
    let operation_id = OperationId::generate(&generator);
    let queue_items = MaxIpcQueueItems::new(2)?;
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Strict,
        queue_items,
    ))
    .await?;
    let batch = session_operation_batch(session_id, operation_id);

    // When: a test failpoint interrupts the batch after its first insert.
    let result = writer.submit_interrupted_batch(batch, 1).await;
    assert!(matches!(result, Err(StorageError::InjectedFailure)));
    writer.shutdown().await?;

    // Then: reopen sees the prior state, never the first row without the second.
    let connection = Connection::open(database)?;
    let sessions: i64 =
        connection.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
    let operations: i64 =
        connection.query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))?;
    assert_eq!((sessions, operations), (0, 0));
    Ok(())
}

fn committed(receipt: WriteReceipt) {
    assert!(matches!(
        receipt,
        WriteReceipt::Committed { rows_changed: 1 }
    ));
}

const fn event_sequence(receipt: WriteReceipt) -> Option<u64> {
    match receipt {
        WriteReceipt::EventAppended { sequence } => Some(sequence),
        WriteReceipt::Committed { .. } | WriteReceipt::BatchCommitted { .. } => None,
    }
}

fn event(
    generator: &UuidV7Generator,
    ids: (SessionId, OperationId),
    payload: &[u8],
) -> WriteCommand {
    let (session_id, operation_id) = ids;
    WriteCommand::Event {
        event_id: EventId::generate(generator),
        session_id: Some(session_id),
        operation_id: Some(operation_id),
        timestamp: "2026-09-08T20:00:03Z".to_owned(),
        event_type: "provider.requested".to_owned(),
        payload: payload.into(),
        schema_version: "1".to_owned(),
    }
}

fn session_operation_batch(session_id: SessionId, operation_id: OperationId) -> WriteBatch {
    WriteBatch::new(WriteCommand::Session {
        session_id,
        started_at: "2026-09-08T20:10:00Z".to_owned(),
        ended_at: None,
        state: SessionState::Active,
        ingress_key: "batch-ingress".to_owned(),
    })
    .and(WriteCommand::Operation {
        operation_id,
        session_id,
        kind: OperationKind::Agent,
        started_at: "2026-09-08T20:10:01Z".to_owned(),
        ended_at: None,
        status: OperationStatus::Started,
    })
}
