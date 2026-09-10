use rusqlite::{Connection, ErrorCode};
use tempfile::TempDir;
use tracepress_core::{
    AttemptId, ContextBlockOccurrenceId, ContextSnapshotId, InferenceStatus, MaxIpcQueueItems,
    OperationId, OperationKind, OperationStatus, RequestId, RequestMetadata, SessionId,
    SessionState, UuidV7Generator,
};

use crate::test_support::TestResult;
use crate::{
    CONTEXT_INSPECTION_MAX_BLOCKS, ContextAnalysisStatus, ContextBlockKind,
    ContextCorrelationStatus, ContextOrigin, ContextRole, Durability, EstimatedTokenComposition,
    EstimatedTokensByKind, EstimatedTokensByOrigin, EstimatedTokensByRole, LogicalContextStatus,
    ReconciliationStatus, StorageConfig, StorageError, StorageWriter, WriteBatch, WriteCommand,
    WriteReceipt,
};

const ALL_KINDS: [ContextBlockKind; 15] = [
    ContextBlockKind::Instructions,
    ContextBlockKind::Message,
    ContextBlockKind::Text,
    ContextBlockKind::ImageReference,
    ContextBlockKind::FileReference,
    ContextBlockKind::ToolDefinition,
    ContextBlockKind::ToolCall,
    ContextBlockKind::ToolResult,
    ContextBlockKind::ItemReference,
    ContextBlockKind::PromptReference,
    ContextBlockKind::ProviderStateReference,
    ContextBlockKind::AssistantHistory,
    ContextBlockKind::OpaqueReasoning,
    ContextBlockKind::Opaque,
    ContextBlockKind::Unknown,
];

const ALL_ROLES: [ContextRole; 6] = [
    ContextRole::System,
    ContextRole::Developer,
    ContextRole::User,
    ContextRole::Assistant,
    ContextRole::Tool,
    ContextRole::Unknown,
];

const ALL_ORIGINS: [ContextOrigin; 8] = [
    ContextOrigin::HumanAuthored,
    ContextOrigin::AgentGenerated,
    ContextOrigin::ToolGenerated,
    ContextOrigin::ToolSchema,
    ContextOrigin::ProviderManaged,
    ContextOrigin::ExternalReference,
    ContextOrigin::TracepressGenerated,
    ContextOrigin::Unknown,
];

/// One foreign-key-valid Phase 2 ancestry for a Phase 3 snapshot.
struct Fixture {
    session: SessionId,
    operation: OperationId,
    request: RequestId,
    attempt: AttemptId,
    snapshot: ContextSnapshotId,
}

fn ancestry(generator: &UuidV7Generator) -> (Fixture, WriteBatch) {
    let fixture = Fixture {
        session: SessionId::generate(generator),
        operation: OperationId::generate(generator),
        request: RequestId::generate(generator),
        attempt: AttemptId::generate(generator),
        snapshot: ContextSnapshotId::generate(generator),
    };
    let batch = WriteBatch::new(WriteCommand::Session {
        session_id: fixture.session,
        started_at: "2026-09-09T09:00:00Z".to_owned(),
        ended_at: None,
        state: SessionState::Active,
        ingress_key: "context-ingress".to_owned(),
    })
    .and(WriteCommand::Operation {
        operation_id: fixture.operation,
        session_id: fixture.session,
        kind: OperationKind::LlmInference,
        started_at: "2026-09-09T09:00:01Z".to_owned(),
        ended_at: None,
        status: OperationStatus::Started,
    })
    .and(WriteCommand::ProviderRequest {
        operation_id: fixture.operation,
        metadata: RequestMetadata::responses(fixture.request, 4_096),
        provider: None,
        protocol: None,
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
    .and(WriteCommand::ProviderAttempt {
        attempt_id: fixture.attempt,
        request_id: fixture.request,
        ordinal: 0,
        status_code: None,
        started_at: "2026-09-09T09:00:02Z".to_owned(),
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
    .and(WriteCommand::ProviderUsage {
        attempt_id: fixture.attempt,
        input_total: Some(1_200),
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
    .and(begin_snapshot(&fixture));
    (fixture, batch)
}

fn begin_snapshot(fixture: &Fixture) -> WriteCommand {
    WriteCommand::ContextSnapshot {
        snapshot_id: fixture.snapshot,
        session_id: fixture.session,
        provider_request_id: fixture.request,
        inference_operation_id: fixture.operation,
        analysis_version: 1,
        status: ContextAnalysisStatus::Partial,
        started_at_us: 1_757_412_000_000_000,
    }
}

fn unknown_block(
    block_occurrence_id: ContextBlockOccurrenceId,
    snapshot_id: ContextSnapshotId,
    ordinal: u64,
) -> WriteCommand {
    WriteCommand::ContextBlockOccurrence {
        block_occurrence_id,
        snapshot_id,
        ordinal,
        parent_block_occurrence_id: None,
        kind: ContextBlockKind::ToolResult,
        role: ContextRole::Tool,
        origin: ContextOrigin::ToolGenerated,
        semantic_path: None,
        semantic_path_truncated: None,
        semantic_path_hash: None,
        raw_value_start: 128,
        raw_value_end: 4_096,
        locator_occurrence: 0,
        raw_bytes: 3_968,
        exact_fingerprint: None,
        semantic_fingerprint: None,
        fingerprint_version: None,
        estimated_tokens: None,
        estimator: None,
        estimator_version: None,
        estimator_encoding: None,
        estimate_confidence: None,
        detected_kind: None,
        detector_confidence: None,
        detector_version: None,
        tool_call_id: None,
        tool_name: None,
        tool_name_truncated: None,
        tool_name_hash: None,
        line_count: None,
        max_line_length: None,
        duplicate_line_ratio: None,
        unique_line_ratio: None,
        json_item_count: None,
        json_depth: None,
        error_line_density: None,
        warning_line_density: None,
        repetition_score: None,
        opportunity_signals: None,
        candidate_estimated_tokens: None,
    }
}

fn unknown_metrics(snapshot_id: ContextSnapshotId) -> WriteCommand {
    WriteCommand::ContextAnalysisMetrics {
        snapshot_id,
        explicit_bytes: None,
        estimated_tokens: Box::new(EstimatedTokenComposition::default()),
        estimated_tool_definition_share: None,
        estimated_tool_result_share: None,
        estimated_human_text_share: None,
        estimated_assistant_history_share: None,
        estimated_unique_content_share: None,
        estimated_repeated_content_share: None,
        tool_count: None,
        schema_bytes: None,
        estimated_schema_tokens: None,
        largest_tool_schema: None,
        repeated_schema_tokens: None,
        stable_explicit_prefix_estimate: None,
        estimator: None,
        estimator_version: None,
        estimate_confidence: None,
        opportunity_signals: None,
    }
}

fn unknown_delta(
    current_snapshot_id: ContextSnapshotId,
    previous_snapshot_id: ContextSnapshotId,
) -> WriteCommand {
    WriteCommand::ContextDelta {
        current_snapshot_id,
        previous_snapshot_id,
        repeated_blocks: None,
        new_blocks: None,
        changed_blocks: None,
        removed_blocks: None,
        repeated_estimated_tokens: None,
        new_estimated_tokens: None,
        common_prefix_blocks: None,
        common_prefix_estimated_tokens: None,
    }
}

async fn open_writer(database: &std::path::Path) -> TestResult<StorageWriter> {
    let writer = StorageWriter::open(StorageConfig::new(
        database.to_path_buf(),
        Durability::Strict,
        MaxIpcQueueItems::new(8)?,
    ))
    .await?;
    Ok(writer)
}

fn optional_numeric_columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT name FROM pragma_table_info(?1) WHERE \"notnull\" = 0 AND type IN ('INTEGER', 'REAL')",
    )?;
    statement
        .query_map([table], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn assert_every_optional_numeric_is_null(connection: &Connection, table: &str) -> TestResult {
    let columns = optional_numeric_columns(connection, table)?;
    assert!(
        !columns.is_empty(),
        "{table} declares no optional numeric column, so missingness cannot be preserved"
    );
    let predicate = columns
        .iter()
        .map(|column| format!("{column} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let populated: i64 = connection.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        populated, 0,
        "{table} substituted a value for an unknown numeric instead of NULL"
    );
    Ok(())
}

fn assert_constraint_violation(result: &Result<WriteReceipt, StorageError>) {
    assert!(
        matches!(
            result,
            Err(StorageError::Sqlite(error))
                if error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation)
        ),
        "expected a foreign-key rejection, observed {result:?}"
    );
}

#[tokio::test]
async fn unknown_context_measurements_persist_as_null_not_zero() -> TestResult {
    // Given: a migrated database with one snapshot whose ancestry is committed.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-null.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let previous_snapshot_id = fixture.snapshot;
    let current_snapshot_id = ContextSnapshotId::generate(&generator);
    let _setup = writer.submit_batch(batch).await?;
    let _second = writer
        .submit(WriteCommand::ContextSnapshot {
            snapshot_id: current_snapshot_id,
            session_id: fixture.session,
            provider_request_id: fixture.request,
            inference_operation_id: fixture.operation,
            analysis_version: 2,
            status: ContextAnalysisStatus::Partial,
            started_at_us: 1_757_412_000_000_001,
        })
        .await?;

    // When: every optional measurement of every Phase 3 record is unknown.
    let _blocks = writer
        .submit(unknown_block(
            ContextBlockOccurrenceId::generate(&generator),
            previous_snapshot_id,
            0,
        ))
        .await?;
    let _metrics = writer.submit(unknown_metrics(previous_snapshot_id)).await?;
    let _delta = writer
        .submit(unknown_delta(current_snapshot_id, previous_snapshot_id))
        .await?;
    let _reconciliation = writer
        .submit(WriteCommand::TokenReconciliation {
            snapshot_id: previous_snapshot_id,
            attempt_id: None,
            visible_estimated_tokens: None,
            provider_input_tokens: None,
            residual_tokens: None,
            comparability: ReconciliationStatus::MissingProviderUsage,
        })
        .await?;
    writer.shutdown().await?;

    // Then: no unknown numeric was substituted by a zero in any Phase 3 table.
    let connection = Connection::open(database)?;
    for table in [
        "context_snapshots",
        "context_block_occurrences",
        "context_analysis_metrics",
        "context_deltas",
        "token_reconciliations",
    ] {
        assert_every_optional_numeric_is_null(&connection, table)?;
    }
    Ok(())
}

#[tokio::test]
async fn negative_residual_tokens_persist_unchanged() -> TestResult {
    // Given: a snapshot whose local estimate exceeded the provider's reported input.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-residual.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let _setup = writer.submit_batch(batch).await?;

    // When: the signed residual is reconciled against the observed provider usage.
    let receipt = writer
        .submit(WriteCommand::TokenReconciliation {
            snapshot_id: fixture.snapshot,
            attempt_id: Some(fixture.attempt),
            visible_estimated_tokens: Some(1_500),
            provider_input_tokens: Some(1_200),
            residual_tokens: Some(-300),
            comparability: ReconciliationStatus::ComparableApproximate,
        })
        .await?;
    writer.shutdown().await?;

    // Then: the negative residual survives verbatim instead of being clamped.
    assert!(matches!(
        receipt,
        WriteReceipt::Committed { rows_changed: 1 }
    ));
    let connection = Connection::open(database)?;
    let residual: i64 = connection.query_row(
        "SELECT residual_tokens FROM token_reconciliations WHERE snapshot_id = ?1",
        [fixture.snapshot.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(residual, -300);
    Ok(())
}

#[tokio::test]
async fn orphan_context_records_are_rejected_by_foreign_keys() -> TestResult {
    // Given: a migrated database holding exactly one committed snapshot.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-orphan.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let _setup = writer.submit_batch(batch).await?;
    let missing_snapshot_id = ContextSnapshotId::generate(&generator);

    // When: each Phase 3 record names a snapshot or usage row that does not exist.
    let block = writer
        .submit(unknown_block(
            ContextBlockOccurrenceId::generate(&generator),
            missing_snapshot_id,
            0,
        ))
        .await;
    let metrics = writer.submit(unknown_metrics(missing_snapshot_id)).await;
    let delta = writer
        .submit(unknown_delta(missing_snapshot_id, fixture.snapshot))
        .await;
    let reconciliation = writer
        .submit(WriteCommand::TokenReconciliation {
            snapshot_id: missing_snapshot_id,
            attempt_id: None,
            visible_estimated_tokens: None,
            provider_input_tokens: None,
            residual_tokens: None,
            comparability: ReconciliationStatus::NotComparable,
        })
        .await;
    let unbacked_usage = writer
        .submit(WriteCommand::TokenReconciliation {
            snapshot_id: fixture.snapshot,
            attempt_id: Some(AttemptId::generate(&generator)),
            visible_estimated_tokens: None,
            provider_input_tokens: None,
            residual_tokens: None,
            comparability: ReconciliationStatus::MissingProviderUsage,
        })
        .await;
    let mut orphan_parent_block = unknown_block(
        ContextBlockOccurrenceId::generate(&generator),
        fixture.snapshot,
        0,
    );
    if let WriteCommand::ContextBlockOccurrence {
        parent_block_occurrence_id,
        ..
    } = &mut orphan_parent_block
    {
        *parent_block_occurrence_id = Some(ContextBlockOccurrenceId::generate(&generator));
    }
    let orphan_parent = writer.submit(orphan_parent_block).await;

    // Then: every orphan is refused and no Phase 3 row survives.
    assert_constraint_violation(&block);
    assert_constraint_violation(&metrics);
    assert_constraint_violation(&delta);
    assert_constraint_violation(&reconciliation);
    assert_constraint_violation(&unbacked_usage);
    assert_constraint_violation(&orphan_parent);
    writer.shutdown().await?;
    let connection = Connection::open(database)?;
    let counts: (i64, i64, i64, i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM context_block_occurrences), (SELECT COUNT(*) FROM context_analysis_metrics), (SELECT COUNT(*) FROM context_deltas), (SELECT COUNT(*) FROM token_reconciliations)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(counts, (0, 0, 0, 0));
    Ok(())
}

#[tokio::test]
async fn a_snapshot_only_becomes_complete_when_its_outcome_commits() -> TestResult {
    // Given: an analysis that has begun but has not been finalized.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-outcome.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let _setup = writer.submit_batch(batch).await?;
    let connection = Connection::open(&database)?;
    let before: (String, Option<i64>) = connection.query_row(
        "SELECT status, completed_at_us FROM context_snapshots WHERE snapshot_id = ?1",
        [fixture.snapshot.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    // When: the analysis finalizes with its terminal status and visibility.
    let _outcome = writer
        .submit(WriteCommand::ContextSnapshotOutcome {
            snapshot_id: fixture.snapshot,
            status: ContextAnalysisStatus::Complete,
            completed_at_us: Some(1_757_412_000_250_000),
            request_content_hash: Some(Box::new([7_u8; 32])),
            explicit_block_count: Some(3),
            analyzed_bytes: Some(4_096),
            skipped_bytes: Some(0),
            explicit_request_complete: Some(true),
            uses_previous_response: Some(false),
            uses_conversation_state: Some(false),
            uses_item_references: Some(false),
            uses_prompt_reference: Some(false),
            uses_external_files: Some(false),
            uses_external_images: Some(false),
            contains_opaque_items: Some(false),
            logical_context_status: Some(LogicalContextStatus::ExplicitOnly),
            duplicate_key_detected: Some(false),
            reference_resolved_locally: None,
            correlation_status: Some(ContextCorrelationStatus::Correlated),
        })
        .await?;
    writer.shutdown().await?;

    // Then: the snapshot was not complete before finalization and is complete after it.
    assert_eq!(before, ("partial".to_owned(), None));
    let after: (String, Option<i64>, Option<i64>, String, Option<i64>) = connection.query_row(
        "SELECT status, completed_at_us, explicit_block_count, logical_context_status, reference_resolved_locally FROM context_snapshots WHERE snapshot_id = ?1",
        [fixture.snapshot.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    assert_eq!(
        after,
        (
            "complete".to_owned(),
            Some(1_757_412_000_250_000),
            Some(3),
            "explicit_only".to_owned(),
            None
        )
    );
    Ok(())
}

#[tokio::test]
async fn every_composition_bucket_persists_in_its_own_column() -> TestResult {
    // Given: a distinct estimated-token count attributed to every kind, role, and origin.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-composition.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let _setup = writer.submit_batch(batch).await?;
    let mut expected = 1_u64;
    let mut by_kind = EstimatedTokensByKind::new();
    for kind in ALL_KINDS {
        by_kind = by_kind.with(kind, expected);
        expected = expected.saturating_add(1);
    }
    let mut by_role = EstimatedTokensByRole::new();
    for role in ALL_ROLES {
        by_role = by_role.with(role, expected);
        expected = expected.saturating_add(1);
    }
    let mut by_origin = EstimatedTokensByOrigin::new();
    for origin in ALL_ORIGINS {
        by_origin = by_origin.with(origin, expected);
        expected = expected.saturating_add(1);
    }

    // When: the composition is persisted for the snapshot.
    let _metrics = writer
        .submit(WriteCommand::ContextAnalysisMetrics {
            snapshot_id: fixture.snapshot,
            explicit_bytes: Some(4_096),
            estimated_tokens: Box::new(EstimatedTokenComposition::new(by_kind, by_role, by_origin)),
            estimated_tool_definition_share: Some(0.25),
            estimated_tool_result_share: Some(0.5),
            estimated_human_text_share: None,
            estimated_assistant_history_share: None,
            estimated_unique_content_share: None,
            estimated_repeated_content_share: None,
            tool_count: Some(9),
            schema_bytes: Some(2_048),
            estimated_schema_tokens: Some(512),
            largest_tool_schema: Some(700),
            repeated_schema_tokens: None,
            stable_explicit_prefix_estimate: Some(64),
            estimator: Some("structural-heuristic".to_owned()),
            estimator_version: Some(1),
            estimate_confidence: None,
            opportunity_signals: None,
        })
        .await?;
    writer.shutdown().await?;

    // Then: no two buckets share a column, so every attribution is readable back.
    let connection = Connection::open(database)?;
    let mut names = connection.prepare(
        "SELECT name FROM pragma_table_info('context_analysis_metrics') WHERE name LIKE 'estimated_tokens_%'",
    )?;
    let columns = names
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        columns.len(),
        ALL_KINDS.len() + ALL_ROLES.len() + ALL_ORIGINS.len()
    );
    let projection = columns.join(", ");
    let mut statement = connection.prepare(&format!(
        "SELECT {projection} FROM context_analysis_metrics"
    ))?;
    let mut values = statement.query_map([], |row| {
        (0..columns.len())
            .map(|index| row.get::<_, Option<i64>>(index))
            .collect::<rusqlite::Result<Vec<_>>>()
    })?;
    let row = values.next().transpose()?.unwrap_or_default();
    let mut observed = Vec::with_capacity(row.len());
    for (column, value) in columns.iter().zip(row) {
        assert!(value.is_some(), "{column} lost its attributed estimate");
        observed.push(value.unwrap_or_default());
    }
    let mut distinct = observed.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        observed.len(),
        "two composition buckets collided into one column: {observed:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_block_batch_commits_atomically_and_a_repeated_ordinal_rejects_it() -> TestResult {
    // Given: a snapshot that already holds one appended batch of three blocks.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-batch.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let _setup = writer.submit_batch(batch).await?;
    let first = writer
        .submit_batch(
            WriteBatch::new(unknown_block(
                ContextBlockOccurrenceId::generate(&generator),
                fixture.snapshot,
                0,
            ))
            .and(unknown_block(
                ContextBlockOccurrenceId::generate(&generator),
                fixture.snapshot,
                1,
            ))
            .and(unknown_block(
                ContextBlockOccurrenceId::generate(&generator),
                fixture.snapshot,
                2,
            )),
        )
        .await?;

    // When: a second batch repeats an ordinal already taken in that snapshot.
    let second = writer
        .submit_batch(
            WriteBatch::new(unknown_block(
                ContextBlockOccurrenceId::generate(&generator),
                fixture.snapshot,
                3,
            ))
            .and(unknown_block(
                ContextBlockOccurrenceId::generate(&generator),
                fixture.snapshot,
                1,
            )),
        )
        .await;
    writer.shutdown().await?;

    // Then: the first batch is intact and the colliding batch left nothing behind.
    assert!(matches!(
        first,
        WriteReceipt::BatchCommitted { rows_changed: 3 }
    ));
    assert_constraint_violation(&second);
    let connection = Connection::open(database)?;
    let mut statement =
        connection.prepare("SELECT ordinal FROM context_block_occurrences ORDER BY ordinal")?;
    let ordinals = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(ordinals, vec![0, 1, 2]);
    Ok(())
}

#[tokio::test]
async fn an_unrepresentable_context_measurement_is_typed_and_commits_nothing() -> TestResult {
    // Given: a committed snapshot and an estimate above SQLite's signed integer range.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-overflow.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let _setup = writer.submit_batch(batch).await?;
    let mut block = unknown_block(
        ContextBlockOccurrenceId::generate(&generator),
        fixture.snapshot,
        0,
    );
    if let WriteCommand::ContextBlockOccurrence {
        estimated_tokens, ..
    } = &mut block
    {
        *estimated_tokens = Some(u64::MAX);
    }

    // When: the writer is asked to persist it.
    let result = writer.submit(block).await;

    // Then: the failure is typed and no block row was committed.
    assert!(matches!(
        result,
        Err(StorageError::IntegerOverflow {
            field: "estimated_tokens",
            value: u64::MAX
        })
    ));
    writer.shutdown().await?;
    let connection = Connection::open(database)?;
    let blocks: i64 = connection.query_row(
        "SELECT COUNT(*) FROM context_block_occurrences",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(blocks, 0);
    Ok(())
}

#[tokio::test]
async fn context_inspection_returns_only_bounded_largest_blocks() -> TestResult {
    // Given: one provider request with more blocks than the inspection IPC contract permits.
    let directory = TempDir::new()?;
    let database = directory.path().join("context-inspection.sqlite3");
    let generator = UuidV7Generator::new();
    let writer = open_writer(&database).await?;
    let (fixture, batch) = ancestry(&generator);
    let _setup = writer.submit_batch(batch).await?;

    let mut blocks = WriteBatch::new(unknown_block(
        ContextBlockOccurrenceId::generate(&generator),
        fixture.snapshot,
        0,
    ));
    for ordinal in 1..5_000 {
        blocks = blocks.and(unknown_block(
            ContextBlockOccurrenceId::generate(&generator),
            fixture.snapshot,
            ordinal,
        ));
    }
    let _blocks = writer.submit_batch(blocks).await?;

    // When: the daemon-owned writer handles the metadata-only inspection read.
    let inspection = writer.inspect_context(fixture.request).await?;
    assert_eq!(inspection.provider_input_tokens, Some(1_200));

    // Then: the largest-block list and serialized result remain bounded.
    assert_eq!(
        inspection.largest_blocks.len(),
        CONTEXT_INSPECTION_MAX_BLOCKS
    );
    assert_eq!(
        inspection
            .largest_blocks
            .first()
            .ok_or("missing largest block")?
            .raw_bytes,
        3_968
    );
    assert!(serde_json::to_vec(&inspection)?.len() < 32_768);
    writer.shutdown().await?;
    Ok(())
}
