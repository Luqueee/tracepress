use rusqlite::Connection;
use tempfile::TempDir;
use tracepress_core::{
    MaxIpcQueueItems, OperationId, OperationKind, OperationStatus, RequestId, RequestMetadata,
    SessionId, SessionState, UuidV7Generator,
};

use crate::test_support::TestResult;
use crate::{
    Durability, StorageConfig, StorageError, StorageWriter, WriteBatch, WriteCommand, WriteReceipt,
};

#[tokio::test]
async fn integer_overflow_returns_typed_failure_without_partial_commit() -> TestResult {
    // Given: a migrated writer with the provider request's parent records committed.
    let directory = TempDir::new()?;
    let database = directory.path().join("overflow.sqlite3");
    let generator = UuidV7Generator::new();
    let session_id = SessionId::generate(&generator);
    let operation_id = OperationId::generate(&generator);
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Balanced,
        MaxIpcQueueItems::new(2)?,
    ))
    .await?;
    let setup = WriteBatch::new(WriteCommand::Session {
        session_id,
        started_at: "2026-09-08T23:00:00Z".to_owned(),
        ended_at: None,
        state: SessionState::Active,
        ingress_key: "overflow-ingress".to_owned(),
    })
    .and(WriteCommand::Operation {
        operation_id,
        session_id,
        kind: OperationKind::LlmInference,
        started_at: "2026-09-08T23:00:01Z".to_owned(),
        ended_at: None,
        status: OperationStatus::Started,
    });
    let setup_receipt = writer.submit_batch(setup).await?;
    assert!(matches!(
        setup_receipt,
        WriteReceipt::BatchCommitted { rows_changed: 2 }
    ));

    // When: a request byte count exceeds SQLite's signed integer range.
    let result = writer
        .submit(WriteCommand::ProviderRequest {
            operation_id,
            metadata: RequestMetadata::chat_completions(RequestId::generate(&generator), u64::MAX),
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
            request_kind: None,
            compaction_trigger: None,
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
        .await;

    // Then: the writer returns a typed overflow and commits no provider request row.
    assert!(matches!(
        result,
        Err(StorageError::IntegerOverflow {
            field: "request_bytes",
            value: u64::MAX
        })
    ));
    writer.shutdown().await?;
    let connection = Connection::open(database)?;
    let requests: i64 =
        connection.query_row("SELECT COUNT(*) FROM provider_requests", [], |row| {
            row.get(0)
        })?;
    assert_eq!(requests, 0);
    Ok(())
}

#[tokio::test]
async fn integer_overflow_rolls_back_earlier_valid_command_in_batch() -> TestResult {
    // Given: one batch containing a valid session followed by an overflowing request.
    let directory = TempDir::new()?;
    let database = directory.path().join("batch-overflow.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Balanced,
        MaxIpcQueueItems::new(2)?,
    ))
    .await?;
    let batch = WriteBatch::new(WriteCommand::Session {
        session_id: SessionId::generate(&generator),
        started_at: "2026-09-08T23:10:00Z".to_owned(),
        ended_at: None,
        state: SessionState::Active,
        ingress_key: "batch-overflow-ingress".to_owned(),
    })
    .and(WriteCommand::ProviderRequest {
        operation_id: OperationId::generate(&generator),
        metadata: RequestMetadata::chat_completions(RequestId::generate(&generator), u64::MAX),
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
        request_kind: None,
        compaction_trigger: None,
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
    });

    // When: the single writer executes the ordered transaction.
    let result = writer.submit_batch(batch).await;

    // Then: the overflow is typed and the preceding session insert is rolled back.
    assert!(matches!(
        result,
        Err(StorageError::IntegerOverflow {
            field: "request_bytes",
            value: u64::MAX
        })
    ));
    writer.shutdown().await?;
    let connection = Connection::open(database)?;
    let rows: (i64, i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM sessions), (SELECT COUNT(*) FROM provider_requests)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(rows, (0, 0));
    Ok(())
}
