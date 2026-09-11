//! Provider-observation persistence tests against a real temporary `SQLite` database.

use rusqlite::Connection;
use tempfile::TempDir;
use tracepress_context::{
    ContextAnalysisDropReason, ContextAnalysisLimitValues, ContextAnalysisLimits,
    ContextAnalysisStatus, TokenReconciliation, analyze,
};
use tracepress_core::{
    ContextSnapshotId, HttpStatusCode, MaxIpcQueueItems, OperationId, OperationKind, RequestId,
    SessionId, SessionState, UuidV7Generator,
};
use tracepress_daemon::{
    ContextAnalysisAbort, ContextAnalysisBegin, ContextAnalysisFinalize, ContextAnalysisMetrics,
    ContextBlockBatch, ContextCorrelationStatusWire, ControlRequest, CorrelationDegradation,
    DaemonError, DaemonService, PersistProviderObservation, ProviderObservation,
    ProviderObservationOutcome, RecordCorrelationDegradation, RecordProviderObservation,
};
use tracepress_provider::{
    ContentEncoding, ObservationInput, ObservationLimits, ProviderRequestKind,
    ProviderResponseState, ProviderTransport, StreamingObserver, parse_request, parse_response,
};
use tracepress_storage::{Durability, StorageConfig, StorageWriter};

/// Provider output text used to prove no event payload carries response content.
const OUTPUT_CANARY: &str = "daemon-event-output-canary-3f81";

/// The exact usage object a streamed fixture reports, which events must never carry.
const RAW_USAGE_OBJECT: &str = r#"{"input_tokens":12,"input_tokens_details":{"cached_tokens":4},"output_tokens":8,"total_tokens":20}"#;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn persists_session_inference_request_attempt_and_usage_causally() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("2026-09-09T00:00:00Z").await?;
    let inference = daemon
        .create_operation(
            session.session_id,
            OperationKind::LlmInference,
            "2026-09-09T00:00:01Z",
            None,
        )
        .await?;
    let response = response(br#"{"id":"resp_1","model":"gpt-test","status":"completed","usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}"#);
    let receipt = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            inference,
            ProviderObservation::new(42, request(), "2026-09-09T00:00:02Z")
                .with_response(response)
                .with_ended_at("2026-09-09T00:00:03Z")
                .with_outcome(ProviderObservationOutcome::Completed),
        ))
        .await?;
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let chain: (i64, i64, i64, i64) = database.query_row(
        "SELECT (SELECT COUNT(*) FROM sessions), (SELECT COUNT(*) FROM operations WHERE operation_id = ?1), (SELECT COUNT(*) FROM provider_requests WHERE operation_id = ?1), (SELECT COUNT(*) FROM provider_attempts WHERE request_id = ?2)",
        rusqlite::params![inference.to_string(), receipt.request_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(chain, (1, 1, 1, 1));
    let route: String = database.query_row(
        "SELECT route FROM provider_requests WHERE operation_id = ?1",
        rusqlite::params![inference.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(route, "responses");
    let usage_attempt: String =
        database.query_row("SELECT attempt_id FROM provider_usage", [], |row| {
            row.get(0)
        })?;
    assert_eq!(usage_attempt, receipt.attempt_id.to_string());
    Ok(())
}

#[tokio::test]
async fn persists_compaction_as_a_dedicated_transport_operation() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;
    let mut request = tracepress_provider::RequestObservation::transport_only(
        ProviderRequestKind::Compaction,
        128,
        128,
        ContentEncoding::Zstd,
        ProviderTransport::ChatGptCodexSubscription,
        1,
    );
    request.wire_sha256 = Some(Box::new([7_u8; 32]));
    let recorded = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(128, request, "t2")
                .with_status_code(HttpStatusCode::new(200)?)
                .with_transport_metrics(64, Some(9))
                .with_outcome(ProviderObservationOutcome::Completed)
                .with_ended_at("t3"),
        ))
        .await?;
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let operation: (String, String) = database.query_row(
        "SELECT kind, status FROM operations WHERE operation_id = ?1",
        [recorded.operation_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(
        operation,
        ("context_compaction".to_owned(), "completed".to_owned())
    );
    let request_row: (String, String, String, String, Option<i64>, Option<i64>) = database.query_row(
        "SELECT route, request_kind, transport, content_encoding, decoded_bytes, previous_response_id_present FROM provider_requests WHERE request_id = ?1",
        [recorded.receipt.request_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    assert_eq!(
        request_row,
        (
            "responses_compact".to_owned(),
            "compaction".to_owned(),
            "chatgpt_codex_subscription".to_owned(),
            "zstd".to_owned(),
            None,
            None,
        )
    );
    let attempt: (i64, i64, i64) = database.query_row(
        "SELECT status_code, byte_count, duration_us FROM provider_attempts WHERE attempt_id = ?1",
        [recorded.receipt.attempt_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(attempt, (200, 64, 9));
    let snapshots: i64 =
        database.query_row("SELECT COUNT(*) FROM context_snapshots", [], |row| {
            row.get(0)
        })?;
    assert_eq!(snapshots, 0);
    Ok(())
}

#[tokio::test]
async fn retries_share_request_identity_and_increment_attempt_ordinal() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("2026-09-09T00:00:00Z").await?;
    let inference = daemon
        .create_operation(
            session.session_id,
            OperationKind::LlmInference,
            "2026-09-09T00:00:01Z",
            None,
        )
        .await?;
    let first = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            inference,
            ProviderObservation::new(10, request(), "t1")
                .with_response(response(br#"{"status":"completed"}"#)),
        ))
        .await?;
    let second = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            inference,
            ProviderObservation::new(10, request(), "t2")
                .with_response(response(br#"{"status":"incomplete"}"#)),
        ))
        .await?;
    assert_eq!(first.request_id, second.request_id);
    assert_ne!(first.attempt_id, second.attempt_id);
    assert_eq!((first.ordinal, second.ordinal), (0, 1));
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let counts: (i64, i64) = database.query_row(
        "SELECT (SELECT COUNT(*) FROM provider_requests), (SELECT COUNT(*) FROM provider_attempts)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(counts, (1, 2));
    Ok(())
}

#[tokio::test]
async fn missing_usage_is_retained_as_null_values() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let inference = daemon
        .create_operation(session.session_id, OperationKind::LlmInference, "t1", None)
        .await?;
    let receipt = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            inference,
            ProviderObservation::new(10, request(), "t2")
                .with_response(response(br#"{"status":"completed"}"#)),
        ))
        .await?;
    assert_eq!(receipt.ordinal, 0);
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let values: (Option<i64>, Option<i64>, String) = database.query_row(
        "SELECT input_total, total, usage_status FROM provider_usage",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(values, (None, None, "unavailable".to_owned()));
    Ok(())
}

#[tokio::test]
async fn rejects_orphan_and_wrong_kind_provider_observations() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let agent = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;
    let unknown = OperationId::generate(&UuidV7Generator::new());
    let orphan = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            unknown,
            ProviderObservation::new(1, request(), "t2"),
        ))
        .await;
    assert!(matches!(orphan, Err(DaemonError::UnknownOperation { .. })));
    let wrong_kind = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            agent,
            ProviderObservation::new(1, request(), "t3"),
        ))
        .await;
    assert!(matches!(
        wrong_kind,
        Err(DaemonError::OperationNotInference { .. })
    ));
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the scenario asserts a complete bounded protocol lifecycle"
)]
async fn context_batches_are_bounded_and_preserve_five_thousand_blocks() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let inference = daemon
        .create_operation(session.session_id, OperationKind::LlmInference, "t1", None)
        .await?;
    let receipt = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            inference,
            ProviderObservation::new(1, request(), "t2")
                .with_response(response(br#"{"status":"completed"}"#)),
        ))
        .await?;
    let snapshot = daemon
        .begin_context_analysis(ContextAnalysisBegin::new(
            session.session_id,
            receipt.request_id,
            inference,
            1,
            1,
        ))
        .await?;
    let analysis = analyze(
        br#"{"input":[{"role":"user","content":[{"type":"input_text","text":"x"}]}]}"#,
        context_limits()?,
    );
    let template = analysis
        .blocks
        .first()
        .cloned()
        .ok_or("fixture must produce one context block")?;
    let mut sequence = 0_u32;
    let mut next_ordinal = 0_u32;
    let mut max_serialized_body = 0_usize;
    let mut batch = Vec::new();
    for ordinal in 0..5_000_u32 {
        let mut block = template.clone();
        block.ordinal = ordinal;
        block.parent_ordinal = None;
        let mut candidate = batch.clone();
        candidate.push(block.clone());
        let encoded = serde_json::to_vec(&ControlRequest::AppendContextBlocks {
            snapshot_id: snapshot,
            sequence,
            blocks: candidate,
        })?;
        max_serialized_body = max_serialized_body.max(encoded.len());
        assert!(
            encoded.len() <= 32_768,
            "one append exceeded the bounded body: {} bytes",
            encoded.len()
        );
        if encoded.len() > 30_000 && !batch.is_empty() {
            let _receipt = daemon
                .append_context_blocks(ContextBlockBatch::new(
                    snapshot,
                    sequence,
                    std::mem::take(&mut batch),
                ))
                .await?;
            sequence = sequence.saturating_add(1);
            let singleton = serde_json::to_vec(&ControlRequest::AppendContextBlocks {
                snapshot_id: snapshot,
                sequence,
                blocks: vec![block.clone()],
            })?;
            max_serialized_body = max_serialized_body.max(singleton.len());
            assert!(singleton.len() <= 32_768);
        }
        batch.push(block);
        next_ordinal = next_ordinal.saturating_add(1);
    }
    if !batch.is_empty() {
        let _receipt = daemon
            .append_context_blocks(ContextBlockBatch::new(snapshot, sequence, batch))
            .await?;
    }
    assert_eq!(next_ordinal, 5_000);
    assert!(
        sequence > 0,
        "5000 blocks must span multiple append requests"
    );
    assert!(max_serialized_body <= 32_768);
    daemon
        .finalize_context_analysis(
            ContextAnalysisFinalize::builder(snapshot, ContextAnalysisStatus::Complete, 2)
                .request_content_hash(Some(analysis.request_content_hash))
                .explicit_block_count(Some(5_000))
                .analyzed_bytes(Some(analysis.analyzed_bytes))
                .skipped_bytes(Some(analysis.skipped_bytes))
                .visibility(analysis.visibility)
                .duplicate_key_detected(Some(analysis.duplicate_key_detected))
                .reference_resolved_locally(Some(
                    analysis.visibility_facts.reference_resolved_locally,
                ))
                .correlation_status(ContextCorrelationStatusWire::Correlated)
                .metrics(empty_context_metrics())
                .delta(None)
                .reconciliation(TokenReconciliation::reconcile(
                    snapshot,
                    analysis.visibility,
                    None,
                    None,
                    false,
                ))
                .attempt_id(Some(receipt.attempt_id))
                .build()?,
        )
        .await?;
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let blocks: i64 = database.query_row(
        "SELECT COUNT(*) FROM context_block_occurrences WHERE snapshot_id = ?1",
        [snapshot.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(blocks, 5_000);
    let snapshot_state: (String, Option<i64>) = database.query_row(
        "SELECT status, completed_at_us FROM context_snapshots WHERE snapshot_id = ?1",
        [snapshot.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(snapshot_state, ("complete".to_owned(), Some(2)));
    Ok(())
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the scenario covers every rejected batch and recovery transition"
)]
async fn context_protocol_rejects_bad_batches_and_recovers_unfinalized_snapshots() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let other_session = daemon.start_session("t0-other").await?;
    let inference = daemon
        .create_operation(session.session_id, OperationKind::LlmInference, "t1", None)
        .await?;
    let receipt = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            inference,
            ProviderObservation::new(1, request(), "t2")
                .with_response(response(br#"{"status":"completed"}"#)),
        ))
        .await?;
    let unknown_session = SessionId::generate(&UuidV7Generator::new());
    let orphan_begin = daemon
        .begin_context_analysis(ContextAnalysisBegin::new(
            unknown_session,
            receipt.request_id,
            inference,
            1,
            1,
        ))
        .await;
    assert!(matches!(
        orphan_begin,
        Err(DaemonError::UnknownSession { .. })
    ));
    let wrong_parent = daemon
        .begin_context_analysis(ContextAnalysisBegin::new(
            other_session.session_id,
            receipt.request_id,
            inference,
            1,
            1,
        ))
        .await;
    assert!(matches!(
        wrong_parent,
        Err(DaemonError::InvalidContextAssociation { .. })
    ));

    let snapshot = daemon
        .begin_context_analysis(ContextAnalysisBegin::new(
            session.session_id,
            receipt.request_id,
            inference,
            1,
            1,
        ))
        .await?;
    let analysis = analyze(
        br#"{"input":[{"role":"user","content":[{"type":"input_text","text":"x"}]}]}"#,
        context_limits()?,
    );
    let template = analysis
        .blocks
        .first()
        .cloned()
        .ok_or("fixture must produce one context block")?;
    let mut child = template.clone();
    child.ordinal = 0;
    child.parent_ordinal = Some(1);
    let mut parent = template.clone();
    parent.ordinal = 1;
    parent.parent_ordinal = None;
    let bad_sequence = daemon
        .append_context_blocks(ContextBlockBatch::new(snapshot, 1, vec![template.clone()]))
        .await;
    assert!(matches!(
        bad_sequence,
        Err(DaemonError::ContextSequence { .. })
    ));
    let bad_parent = daemon
        .append_context_blocks(ContextBlockBatch::new(snapshot, 0, vec![child, parent]))
        .await;
    assert!(matches!(
        bad_parent,
        Err(DaemonError::InvalidContextBlock { .. })
    ));
    let orphan_append = daemon
        .append_context_blocks(ContextBlockBatch::new(
            ContextSnapshotId::generate(&UuidV7Generator::new()),
            0,
            vec![template.clone()],
        ))
        .await;
    assert!(matches!(
        orphan_append,
        Err(DaemonError::UnknownContextSnapshot { .. })
    ));
    let mut first = template.clone();
    first.ordinal = 0;
    first.parent_ordinal = None;
    let _receipt = daemon
        .append_context_blocks(ContextBlockBatch::new(snapshot, 0, vec![first]))
        .await?;
    daemon
        .finalize_context_analysis(
            ContextAnalysisFinalize::builder(snapshot, ContextAnalysisStatus::Partial, 2)
                .request_content_hash(Some(analysis.request_content_hash))
                .explicit_block_count(Some(1))
                .analyzed_bytes(Some(analysis.analyzed_bytes))
                .skipped_bytes(Some(analysis.skipped_bytes))
                .visibility(analysis.visibility)
                .duplicate_key_detected(Some(analysis.duplicate_key_detected))
                .reference_resolved_locally(Some(
                    analysis.visibility_facts.reference_resolved_locally,
                ))
                .correlation_status(ContextCorrelationStatusWire::Correlated)
                .metrics(empty_context_metrics())
                .delta(None)
                .reconciliation(TokenReconciliation::reconcile(
                    snapshot,
                    analysis.visibility,
                    None,
                    None,
                    false,
                ))
                .attempt_id(Some(receipt.attempt_id))
                .build()?,
        )
        .await?;
    let duplicate_finalize = daemon
        .finalize_context_analysis(
            ContextAnalysisFinalize::builder(snapshot, ContextAnalysisStatus::Partial, 3)
                .explicit_block_count(Some(1))
                .visibility(analysis.visibility)
                .correlation_status(ContextCorrelationStatusWire::Correlated)
                .metrics(empty_context_metrics())
                .delta(None)
                .reconciliation(TokenReconciliation::reconcile(
                    snapshot,
                    analysis.visibility,
                    None,
                    None,
                    false,
                ))
                .build()?,
        )
        .await;
    assert!(matches!(
        duplicate_finalize,
        Err(DaemonError::UnknownContextSnapshot { .. })
    ));

    let second_inference = daemon
        .create_operation(session.session_id, OperationKind::LlmInference, "t3", None)
        .await?;
    let second_receipt = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session.session_id,
            second_inference,
            ProviderObservation::new(2, request(), "t4")
                .with_response(response(br#"{"status":"completed"}"#)),
        ))
        .await?;
    let unfinished = daemon
        .begin_context_analysis(ContextAnalysisBegin::new(
            session.session_id,
            second_receipt.request_id,
            second_inference,
            1,
            4,
        ))
        .await?;
    let _receipt = daemon
        .append_context_blocks(ContextBlockBatch::new(unfinished, 0, vec![template]))
        .await?;
    daemon.shutdown().await?;

    let writer = StorageWriter::open(StorageConfig::new(
        directory.path().join("tracepress.sqlite3"),
        Durability::Strict,
        MaxIpcQueueItems::new(16)?,
    ))
    .await?;
    let reopened = DaemonService::open(writer, "reopened").await?;
    reopened.shutdown().await?;
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let recovered: (String, Option<i64>) = database.query_row(
        "SELECT status, completed_at_us FROM context_snapshots WHERE snapshot_id = ?1",
        [unfinished.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(recovered, ("partial".to_owned(), None));
    Ok(())
}

#[tokio::test]
async fn exact_append_caps_remain_finalizable_and_reject_a_suffix() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("exact-caps").await?;
    let snapshot = begin_active_context(&daemon, session.session_id, 1).await?;
    let analysis = analyze(
        br#"{"input":[{"role":"user","content":[{"type":"input_text","text":"x"}]}]}"#,
        context_limits()?,
    );
    let template = analysis
        .blocks
        .first()
        .cloned()
        .ok_or("fixture must produce one context block")?;
    let max_batches = u32::try_from(ContextAnalysisLimits::MAX_BATCHES)?;
    let blocks_per_batch =
        usize::try_from(ContextAnalysisLimits::MAX_BLOCKS / ContextAnalysisLimits::MAX_BATCHES)?;
    let mut last_receipt = None;
    for sequence in 0..max_batches {
        let mut blocks = Vec::with_capacity(blocks_per_batch);
        for offset in 0..blocks_per_batch {
            let mut block = template.clone();
            block.ordinal = sequence
                .saturating_mul(u32::try_from(blocks_per_batch)?)
                .saturating_add(u32::try_from(offset)?);
            block.parent_ordinal = None;
            blocks.push(block);
        }
        last_receipt = Some(
            daemon
                .append_context_blocks(ContextBlockBatch::new(snapshot, sequence, blocks))
                .await?,
        );
    }
    let last_receipt = last_receipt.ok_or("max_batches must be non-zero")?;
    assert_eq!(
        last_receipt.accepted_block_count,
        ContextAnalysisLimits::MAX_BLOCKS
    );
    assert_eq!(last_receipt.next_sequence, max_batches);
    assert_eq!(
        last_receipt.capacity,
        tracepress_daemon::ContextAppendCapacity::BothExhausted
    );
    assert_eq!(last_receipt.status, ContextAnalysisStatus::Partial);

    let mut extra = template;
    extra.ordinal = u32::try_from(ContextAnalysisLimits::MAX_BLOCKS)?;
    let rejected = daemon
        .append_context_blocks(ContextBlockBatch::new(snapshot, max_batches, vec![extra]))
        .await;
    assert!(matches!(
        rejected,
        Err(DaemonError::ContextLimit {
            dimension: "batches",
            ..
        })
    ));

    let mut finalize = minimal_context_finalize(snapshot, ContextAnalysisStatus::Complete, 2)?;
    finalize.explicit_block_count = Some(ContextAnalysisLimits::MAX_BLOCKS);
    daemon.finalize_context_analysis(finalize).await?;
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let block_count: i64 = database.query_row(
        "SELECT COUNT(*) FROM context_block_occurrences WHERE snapshot_id = ?1",
        rusqlite::params![snapshot.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(
        block_count,
        i64::try_from(ContextAnalysisLimits::MAX_BLOCKS)?
    );
    let status: String = database.query_row(
        "SELECT status FROM context_snapshots WHERE snapshot_id = ?1",
        rusqlite::params![snapshot.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(status, "complete");
    Ok(())
}
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the scenario fills the bounded active table and proves deterministic session cleanup"
)]
async fn finishing_a_session_partials_every_active_context_and_releases_capacity() -> TestResult {
    const ACTIVE_LIMIT: usize = 64;
    const ACTIVE_LIMIT_U64: u64 = 64;
    const ACTIVE_LIMIT_I64: i64 = 64;
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("finish-active").await?;
    let mut snapshots = Vec::with_capacity(ACTIVE_LIMIT);
    for ordinal in 0_u64..ACTIVE_LIMIT_U64 {
        snapshots.push(begin_active_context(&daemon, session.session_id, ordinal).await?);
    }

    let (operation_id, provider_request_id) =
        provider_identity(&daemon, session.session_id, ACTIVE_LIMIT_U64).await?;
    let denied = daemon
        .begin_context_analysis(ContextAnalysisBegin::new(
            session.session_id,
            provider_request_id,
            operation_id,
            1,
            ACTIVE_LIMIT_U64,
        ))
        .await;
    assert!(matches!(
        denied,
        Err(DaemonError::ContextAnalysisCapacity {
            limit: ACTIVE_LIMIT
        })
    ));

    let _snapshot = daemon
        .finish_session(
            session.session_id,
            SessionState::Closed,
            "finish-active-ended",
        )
        .await?;
    let replacement = daemon.start_session("replacement").await?;
    let replacement_snapshot =
        begin_active_context(&daemon, replacement.session_id, ACTIVE_LIMIT_U64 + 1).await?;
    daemon
        .abort_context_analysis(ContextAnalysisAbort::new(
            replacement_snapshot,
            ContextAnalysisDropReason::Cancelled,
            100,
        ))
        .await?;
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let completed_partials: i64 = database.query_row(
        "SELECT COUNT(*) FROM context_snapshots WHERE status = 'partial' AND completed_at_us IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(completed_partials, ACTIVE_LIMIT_I64);
    assert_eq!(
        event_count(&database, "context.analysis.partial")?,
        ACTIVE_LIMIT_I64
    );
    assert_eq!(event_count(&database, "context.analysis.dropped")?, 1);
    let unfinished: i64 = database.query_row(
        "SELECT COUNT(*) FROM context_snapshots WHERE completed_at_us IS NULL",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(unfinished, 0);
    assert_eq!(snapshots.len(), ACTIVE_LIMIT);
    Ok(())
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the scenario covers abort retryability, idempotence, and admission after release"
)]
async fn abort_releases_capacity_and_persists_exactly_one_drop_event_after_retry() -> TestResult {
    const ACTIVE_LIMIT: usize = 64;
    const ACTIVE_LIMIT_U64: u64 = 64;
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("abort-active").await?;
    let mut snapshots = Vec::with_capacity(ACTIVE_LIMIT);
    for ordinal in 0_u64..ACTIVE_LIMIT_U64 {
        snapshots.push(begin_active_context(&daemon, session.session_id, ordinal).await?);
    }
    let aborted = *snapshots.first().ok_or("missing first context snapshot")?;
    daemon
        .abort_context_analysis(ContextAnalysisAbort::new(
            aborted,
            ContextAnalysisDropReason::Cancelled,
            10,
        ))
        .await?;
    let duplicate_abort = daemon
        .abort_context_analysis(ContextAnalysisAbort::new(
            aborted,
            ContextAnalysisDropReason::Cancelled,
            11,
        ))
        .await;
    assert!(matches!(
        duplicate_abort,
        Err(DaemonError::UnknownContextSnapshot { snapshot_id }) if snapshot_id == aborted
    ));
    let duplicate_finalize = daemon
        .finalize_context_analysis(minimal_context_finalize(
            aborted,
            ContextAnalysisStatus::Partial,
            12,
        )?)
        .await;
    assert!(matches!(
        duplicate_finalize,
        Err(DaemonError::UnknownContextSnapshot { snapshot_id }) if snapshot_id == aborted
    ));

    let retryable = *snapshots.get(1).ok_or("missing second context snapshot")?;
    let failed_abort = daemon
        .abort_context_analysis(ContextAnalysisAbort::new(
            retryable,
            ContextAnalysisDropReason::ResourceLimit,
            u64::MAX,
        ))
        .await;
    assert!(matches!(failed_abort, Err(DaemonError::Storage(_))));
    daemon
        .abort_context_analysis(ContextAnalysisAbort::new(
            retryable,
            ContextAnalysisDropReason::ResourceLimit,
            20,
        ))
        .await?;

    let replacement =
        begin_active_context(&daemon, session.session_id, ACTIVE_LIMIT_U64 + 1).await?;
    daemon
        .abort_context_analysis(ContextAnalysisAbort::new(
            replacement,
            ContextAnalysisDropReason::Cancelled,
            21,
        ))
        .await?;
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    assert_eq!(event_count(&database, "context.analysis.dropped")?, 3);
    let statuses: (String, String) = database.query_row(
        "SELECT (SELECT status FROM context_snapshots WHERE snapshot_id = ?1), (SELECT status FROM context_snapshots WHERE snapshot_id = ?2)",
        rusqlite::params![aborted.to_string(), retryable.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(
        statuses,
        ("cancelled".to_owned(), "resource_limit".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn finishing_one_session_leaves_another_context_analysis_finalizable() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let first = daemon.start_session("first").await?;
    let second = daemon.start_session("second").await?;
    let first_snapshot = begin_active_context(&daemon, first.session_id, 1).await?;
    let second_snapshot = begin_active_context(&daemon, second.session_id, 2).await?;

    let _snapshot = daemon
        .finish_session(first.session_id, SessionState::Closed, "first-ended")
        .await?;
    daemon
        .finalize_context_analysis(minimal_context_finalize(
            second_snapshot,
            ContextAnalysisStatus::Complete,
            30,
        )?)
        .await?;
    let duplicate_first = daemon
        .finalize_context_analysis(minimal_context_finalize(
            first_snapshot,
            ContextAnalysisStatus::Partial,
            31,
        )?)
        .await;
    assert!(matches!(
        duplicate_first,
        Err(DaemonError::UnknownContextSnapshot { snapshot_id }) if snapshot_id == first_snapshot
    ));
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let statuses: (String, String) = database.query_row(
        "SELECT (SELECT status FROM context_snapshots WHERE snapshot_id = ?1), (SELECT status FROM context_snapshots WHERE snapshot_id = ?2)",
        rusqlite::params![first_snapshot.to_string(), second_snapshot.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(statuses, ("partial".to_owned(), "complete".to_owned()));
    assert_eq!(event_count(&database, "context.analysis.partial")?, 1);
    assert_eq!(event_count(&database, "context.analysis.completed")?, 1);
    Ok(())
}

#[tokio::test]
async fn restarting_recovers_unfinished_context_once_and_releases_capacity() -> TestResult {
    let directory = TempDir::new()?;
    let first = daemon(&directory).await?;
    let session = first.start_session("interrupted").await?;
    let snapshot = begin_active_context(&first, session.session_id, 1).await?;
    first.shutdown().await?;

    let recovered = daemon(&directory).await?;
    let replacement = recovered.start_session("replacement").await?;
    let replacement_snapshot = begin_active_context(&recovered, replacement.session_id, 2).await?;
    recovered
        .abort_context_analysis(ContextAnalysisAbort::new(
            replacement_snapshot,
            ContextAnalysisDropReason::Cancelled,
            20,
        ))
        .await?;
    recovered.shutdown().await?;

    let reopened = daemon(&directory).await?;
    reopened.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let status: String = database.query_row(
        "SELECT status FROM context_snapshots WHERE snapshot_id = ?1",
        rusqlite::params![snapshot.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(status, "partial");
    assert_eq!(event_count(&database, "context.analysis.partial")?, 1);
    Ok(())
}

#[tokio::test]
async fn rejects_correlation_degradations_of_sessions_it_does_not_own() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let unknown = SessionId::generate(&UuidV7Generator::new());
    let orphan = daemon
        .record_correlation_degradation(RecordCorrelationDegradation::new(
            unknown,
            CorrelationDegradation::RetiredLimit,
            "t1".to_owned(),
        ))
        .await;
    assert!(matches!(orphan, Err(DaemonError::UnknownSession { .. })));
    let _closed = daemon
        .finish_session(session.session_id, SessionState::Closed, "t2")
        .await?;
    let late = daemon
        .record_correlation_degradation(RecordCorrelationDegradation::new(
            session.session_id,
            CorrelationDegradation::InFlightLimit,
            "t3".to_owned(),
        ))
        .await;
    assert!(matches!(late, Err(DaemonError::SessionNotActive { .. })));
    daemon.shutdown().await?;

    // A refused degradation is not a degradation this daemon observed: inventing an event for
    // one would attribute a correlation loss to a session that never reported it.
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let events: i64 = database.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'context.correlation.degraded'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(events, 0);
    Ok(())
}

#[tokio::test]
async fn recording_creates_one_inference_beneath_the_root_operation() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;
    let recorded = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(42, request(), "t2")
                .with_response(response(
                    br#"{"id":"resp_1","model":"gpt-test","status":"completed","usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}"#,
                ))
                .with_ended_at("t3")
                .with_outcome(ProviderObservationOutcome::Completed),
        ))
        .await?;
    assert_eq!(recorded.receipt.ordinal, 0);
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let kind: String = database.query_row(
        "SELECT kind FROM operations WHERE operation_id = ?1",
        rusqlite::params![recorded.operation_id.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(kind, "llm_inference");
    let edge: (String, String) = database.query_row(
        "SELECT parent_operation_id, relationship FROM causal_edges WHERE child_operation_id = ?1",
        rusqlite::params![recorded.operation_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(edge, (root.to_string(), "spawned".to_owned()));
    let counts: (i64, i64, i64) = database.query_row(
        "SELECT (SELECT COUNT(*) FROM provider_requests), (SELECT COUNT(*) FROM provider_attempts), (SELECT COUNT(*) FROM provider_usage)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(counts, (1, 1, 1));
    Ok(())
}

#[tokio::test]
async fn recording_rejects_unknown_sessions_and_roots_without_creating_operations() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let ids = UuidV7Generator::new();
    let session = daemon.start_session("t0").await?;
    let unknown_root = OperationId::generate(&ids);
    let orphan = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            unknown_root,
            ProviderObservation::new(1, request(), "t1"),
        ))
        .await;
    assert!(matches!(orphan, Err(DaemonError::UnknownOperation { .. })));
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;
    let _closed_session = daemon
        .finish_session(session.session_id, SessionState::Closed, "t2")
        .await?;
    let closed = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(1, request(), "t3"),
        ))
        .await;
    assert!(matches!(closed, Err(DaemonError::SessionNotActive { .. })));
    daemon.shutdown().await?;

    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let inferences: i64 = database.query_row(
        "SELECT COUNT(*) FROM operations WHERE kind = 'llm_inference'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(inferences, 0);
    Ok(())
}

#[tokio::test]
async fn a_rate_limited_error_body_persists_an_errored_attempt_that_closes_its_operation()
-> TestResult {
    // Given: an upstream that answered 429 with a provider error body and no lifecycle state.
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;
    let error_body = response(
        br#"{"error":{"code":"rate_limit_exceeded","message":"slow down"},"type":"error"}"#,
    );
    assert_eq!(error_body.response_state, ProviderResponseState::Unknown);

    // When: the forward is recorded with its observed HTTP status.
    let recorded = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(64, request(), "t2")
                .with_response(error_body)
                .with_status_code(HttpStatusCode::new(429)?)
                .with_ended_at("t3"),
        ))
        .await?;
    daemon.shutdown().await?;

    // Then: the attempt is errored with its error code, never started and never completed.
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let attempt: (String, Option<String>, Option<i64>, Option<String>) = database.query_row(
        "SELECT status, error_code, status_code, ended_at FROM provider_attempts WHERE attempt_id = ?1",
        rusqlite::params![recorded.receipt.attempt_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        attempt,
        (
            "errored".to_owned(),
            Some("rate_limit_exceeded".to_owned()),
            Some(429),
            Some("t3".to_owned())
        )
    );
    // And: the inference operation is closed with the same terminal evidence.
    let operation: (String, Option<String>) = database.query_row(
        "SELECT status, ended_at FROM operations WHERE operation_id = ?1",
        rusqlite::params![recorded.operation_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(operation, ("errored".to_owned(), Some("t3".to_owned())));
    // And: the canonical vocabulary reports the failure, never a completion.
    assert_eq!(event_count(&database, "provider.response.failed")?, 1);
    assert_eq!(event_count(&database, "provider.response.completed")?, 0);
    Ok(())
}

#[tokio::test]
async fn a_transport_failure_persists_its_failure_class_and_closes_the_operation() -> TestResult {
    // Given: a forward whose upstream connection failed before any response existed.
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;

    // When: the forward is recorded with only its transport evidence.
    let recorded = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(64, request(), "t2")
                .with_transport_error("connect")
                .with_ended_at("t3"),
        ))
        .await?;
    daemon.shutdown().await?;

    // Then: the attempt is errored, carries the failure class, and is not left pending.
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let attempt: (String, Option<String>, Option<i64>, Option<String>) = database.query_row(
        "SELECT status, transport_error, status_code, ended_at FROM provider_attempts WHERE attempt_id = ?1",
        rusqlite::params![recorded.receipt.attempt_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        attempt,
        (
            "errored".to_owned(),
            Some("connect".to_owned()),
            None,
            Some("t3".to_owned())
        )
    );
    let operation: (String, Option<String>) = database.query_row(
        "SELECT status, ended_at FROM operations WHERE operation_id = ?1",
        rusqlite::params![recorded.operation_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(operation, ("errored".to_owned(), Some("t3".to_owned())));
    // And: a response that never existed contributes no usage row and no start event.
    assert_eq!(
        table_count(&database, "provider_usage")?,
        0,
        "a forward without a response must not claim usage"
    );
    assert_eq!(event_count(&database, "provider.response.started")?, 0);
    assert_eq!(event_count(&database, "provider.response.failed")?, 1);
    Ok(())
}

#[tokio::test]
async fn measured_stream_timings_persist_and_no_column_holds_a_placeholder_zero() -> TestResult {
    // Given: a real streamed observation whose observer measured its own elapsed time.
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;
    let streamed = streamed_response()?;

    // When: the completed stream is recorded.
    let recorded = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(64, request(), "unix-ms:1788964322689")
                .with_response(streamed)
                .with_ended_at("unix-ms:1788964322844")
                .with_streaming(true),
        ))
        .await?;
    daemon.shutdown().await?;

    // Then: every latency column holds the observer's own measurement.
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let timings: (Option<i64>, Option<i64>, Option<i64>, String) = database.query_row(
        "SELECT ttfb_us, ttft_us, duration_us, status FROM provider_attempts WHERE attempt_id = ?1",
        rusqlite::params![recorded.receipt.attempt_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let (first_byte, first_token, duration, status) = timings;
    assert_eq!(status, "completed");
    let first_byte = first_byte.ok_or("time to first byte must be measured")?;
    let first_token = first_token.ok_or("time to first token must be measured")?;
    let duration = duration.ok_or("duration must be measured")?;
    assert!(
        first_byte <= first_token && first_token <= duration,
        "measured timings must be ordered: {first_byte} <= {first_token} <= {duration}"
    );
    assert!(first_token > 0, "a measured latency is a real elapsed time");
    // And: no text column of the attempt was filled with the former placeholder marker.
    let placeholders: i64 = database.query_row(
        "SELECT COUNT(*) FROM provider_attempts WHERE '0' IN (started_at, ended_at, provider_created_at, incomplete_reason, error_code, transport_error)",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(placeholders, 0, "unknown must be NULL, never a sentinel");
    Ok(())
}

#[tokio::test]
async fn canonical_events_commit_exactly_once_beside_the_relational_rows() -> TestResult {
    // Given: one completed streamed observation carrying provider usage.
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;

    // When: it is recorded in one atomic batch.
    let recorded = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(64, request(), "t2")
                .with_response(streamed_response()?)
                .with_status_code(HttpStatusCode::new(200)?)
                .with_ended_at("t3")
                .with_streaming(true),
        ))
        .await?;
    daemon.shutdown().await?;

    // Then: exactly the events this evidence supports exist, once each.
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let mut statement = database
        .prepare("SELECT event_type, COUNT(*) FROM events GROUP BY event_type ORDER BY 1")?;
    let emitted = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        emitted,
        vec![
            ("provider.request.observed".to_owned(), 1),
            ("provider.response.completed".to_owned(), 1),
            ("provider.response.started".to_owned(), 1),
            ("provider.usage.normalized".to_owned(), 1),
            ("provider.usage.observed".to_owned(), 1),
        ]
    );
    drop(statement);
    // And: every event belongs to the inference operation of the recorded session.
    let attributed: i64 = database.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND operation_id = ?2",
        rusqlite::params![
            session.session_id.to_string(),
            recorded.operation_id.to_string()
        ],
        |row| row.get(0),
    )?;
    assert_eq!(attributed, 5);
    // And: no payload carries provider content, the raw usage bytes, or a canary.
    let mut statement = database.prepare("SELECT event_type, payload FROM events")?;
    let payloads = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (event_type, payload) in payloads {
        let text = String::from_utf8(payload)?;
        for forbidden in [RAW_USAGE_OBJECT, OUTPUT_CANARY, "input_tokens"] {
            assert!(
                !text.contains(forbidden),
                "{event_type} payload leaked {forbidden}: {text}"
            );
        }
        assert!(
            text.contains("attempt_id"),
            "{event_type} payload must identify its attempt: {text}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_disconnected_stream_reports_incomplete_and_partial_without_claiming_completion()
-> TestResult {
    // Given: a stream the upstream dropped before reporting any terminal state.
    let directory = TempDir::new()?;
    let daemon = daemon(&directory).await?;
    let session = daemon.start_session("t0").await?;
    let root = daemon
        .create_operation(session.session_id, OperationKind::Agent, "t1", None)
        .await?;
    let mut observer = StreamingObserver::new(ObservationLimits::default());
    observer.observe_chunk(
        b"event: response.created\ndata: {\"response\":{\"id\":\"resp_cut\",\"status\":\"in_progress\"}}\n\n",
    )?;
    let cut = observer.disconnect();

    // When: the forward is recorded with that terminal transport decision.
    let recorded = daemon
        .record_provider_observation(RecordProviderObservation::new(
            session.session_id,
            root,
            ProviderObservation::new(64, request(), "t2")
                .with_response(cut)
                .with_status_code(HttpStatusCode::new(200)?)
                .with_ended_at("t3")
                .with_outcome(ProviderObservationOutcome::Disconnected)
                .with_streaming(true),
        ))
        .await?;
    daemon.shutdown().await?;

    // Then: the relational row keeps the disconnect distinct from an incomplete provider stop.
    let database = Connection::open(directory.path().join("tracepress.sqlite3"))?;
    let attempt: (String, Option<String>, Option<String>) = database.query_row(
        "SELECT status, response_state, observation_status FROM provider_attempts WHERE attempt_id = ?1",
        rusqlite::params![recorded.receipt.attempt_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        attempt,
        (
            "disconnected".to_owned(),
            Some("disconnected".to_owned()),
            Some("partial".to_owned())
        )
    );
    assert_eq!(
        text(
            &database,
            "SELECT status FROM operations WHERE kind = 'llm_inference'"
        )?,
        "disconnected"
    );
    // And: the canonical vocabulary reports a non-completed response and a partial observation.
    assert_eq!(event_count(&database, "provider.response.incomplete")?, 1);
    assert_eq!(event_count(&database, "provider.observation.partial")?, 1);
    assert_eq!(event_count(&database, "provider.response.completed")?, 0);
    assert_eq!(event_count(&database, "provider.response.failed")?, 0);
    Ok(())
}

async fn provider_identity(
    daemon: &DaemonService,
    session_id: SessionId,
    ordinal: u64,
) -> Result<(OperationId, RequestId), Box<dyn std::error::Error>> {
    let operation_id = daemon
        .create_operation(
            session_id,
            OperationKind::LlmInference,
            &format!("context-operation-{ordinal}"),
            None,
        )
        .await?;
    let receipt = daemon
        .persist_provider_observation(PersistProviderObservation::new(
            session_id,
            operation_id,
            ProviderObservation::new(ordinal, request(), format!("context-request-{ordinal}"))
                .with_response(response(br#"{"status":"completed"}"#)),
        ))
        .await?;
    Ok((operation_id, receipt.request_id))
}

async fn begin_active_context(
    daemon: &DaemonService,
    session_id: SessionId,
    ordinal: u64,
) -> Result<ContextSnapshotId, Box<dyn std::error::Error>> {
    let (operation_id, provider_request_id) =
        provider_identity(daemon, session_id, ordinal).await?;
    Ok(daemon
        .begin_context_analysis(ContextAnalysisBegin::new(
            session_id,
            provider_request_id,
            operation_id,
            1,
            ordinal.saturating_add(1),
        ))
        .await?)
}
fn minimal_context_finalize(
    snapshot_id: ContextSnapshotId,
    status: ContextAnalysisStatus,
    completed_at_us: u64,
) -> Result<ContextAnalysisFinalize, Box<dyn std::error::Error>> {
    let analysis = analyze(
        br#"{"input":[{"role":"user","content":[{"type":"input_text","text":"x"}]}]}"#,
        context_limits()?,
    );
    Ok(
        ContextAnalysisFinalize::builder(snapshot_id, status, completed_at_us)
            .explicit_block_count(Some(0))
            .visibility(analysis.visibility)
            .correlation_status(ContextCorrelationStatusWire::Correlated)
            .metrics(empty_context_metrics())
            .delta(None)
            .reconciliation(TokenReconciliation::reconcile(
                snapshot_id,
                analysis.visibility,
                None,
                None,
                false,
            ))
            .build()?,
    )
}

fn text(database: &Connection, query: &str) -> rusqlite::Result<String> {
    database.query_row(query, [], |row| row.get(0))
}

fn event_count(database: &Connection, event_type: &str) -> rusqlite::Result<i64> {
    database.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = ?1",
        rusqlite::params![event_type],
        |row| row.get(0),
    )
}

fn table_count(database: &Connection, table: &str) -> rusqlite::Result<i64> {
    database.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
}

fn context_limits() -> Result<ContextAnalysisLimits, Box<dyn std::error::Error>> {
    Ok(ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
        max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        max_analysis_work_units: 100_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })?)
}

const fn empty_context_metrics() -> ContextAnalysisMetrics {
    ContextAnalysisMetrics::new()
}

fn request() -> tracepress_provider::RequestObservation {
    parse_request(ObservationInput::new(
        br#"{"model":"gpt-test","stream":true}"#,
        ObservationLimits::default(),
    ))
}

fn response(bytes: &[u8]) -> tracepress_provider::ResponseObservation {
    parse_response(ObservationInput::new(bytes, ObservationLimits::default()))
}

/// Observes a real SSE stream so the returned timings are the observer's own measurements.
fn streamed_response()
-> Result<tracepress_provider::ResponseObservation, Box<dyn std::error::Error>> {
    let mut observer = StreamingObserver::new(ObservationLimits::default());
    let chunks = [
        "event: response.created\ndata: {\"response\":{\"id\":\"resp_stream\",\"model\":\"gpt-test\",\"status\":\"in_progress\"}}\n\n".to_owned(),
        format!("event: response.output_text.delta\ndata: {{\"delta\":\"{OUTPUT_CANARY}\"}}\n\n"),
        format!(
            "event: response.completed\ndata: {{\"response\":{{\"id\":\"resp_stream\",\"model\":\"gpt-test\",\"status\":\"completed\",\"usage\":{RAW_USAGE_OBJECT}}}}}\n\n"
        ),
    ];
    for chunk in chunks {
        // A measurable gap between fragments keeps the recorded latencies strictly ordered.
        std::thread::sleep(std::time::Duration::from_millis(2));
        observer.observe_chunk(chunk.as_bytes())?;
    }
    Ok(observer.finish())
}

async fn daemon(directory: &TempDir) -> Result<DaemonService, Box<dyn std::error::Error>> {
    let writer = StorageWriter::open(StorageConfig::new(
        directory.path().join("tracepress.sqlite3"),
        Durability::Strict,
        MaxIpcQueueItems::new(16)?,
    ))
    .await?;
    Ok(DaemonService::open(writer, "recovery").await?)
}
