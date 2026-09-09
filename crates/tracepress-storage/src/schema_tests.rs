use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use tracepress_core::MaxIpcQueueItems;

use crate::test_support::TestResult;
use crate::{Durability, StorageConfig, StorageError, StorageWriter, schema};

/// Digest of the released v1 migration, which the contract declares immutable.
const MIGRATION_V1_SHA256: &str =
    "4a08ae8e8bfd52f0eb427ccaed02b11bb1d6a12676b11efd77356b7e66acc119";

/// Digest of the released v2 migration, which the contract declares immutable.
const MIGRATION_V2_SHA256: &str =
    "1654365bc55c1e00796df87c1061514d70581fd7873d770654128b7dd824b2ec";

const TABLES: [&str; 20] = [
    "causal_edges",
    "compression_decisions",
    "content_bindings",
    "content_objects",
    "content_occurrences",
    "context_analysis_metrics",
    "context_block_occurrences",
    "context_deltas",
    "context_snapshots",
    "evaluations",
    "events",
    "operations",
    "policy_assignments",
    "provider_attempts",
    "provider_requests",
    "provider_usage",
    "recoveries",
    "schema_metadata",
    "sessions",
    "token_reconciliations",
];

const V3_TABLES: [&str; 5] = [
    "context_analysis_metrics",
    "context_block_occurrences",
    "context_deltas",
    "context_snapshots",
    "token_reconciliations",
];

fn table_columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    statement
        .query_map([], |row| row.get(1))?
        .collect::<rusqlite::Result<Vec<_>>>()
}

const V2_COLUMNS: [(&str, &[&str]); 3] = [
    (
        "provider_requests",
        &[
            "provider",
            "protocol",
            "parser_version",
            "observation_status",
            "model",
            "stream",
            "background",
            "store",
            "reasoning_effort",
            "text_verbosity",
            "truncation",
            "previous_response_id_present",
            "input_item_count",
            "tool_count",
            "text_input_block_count",
            "image_input_block_count",
            "file_input_block_count",
        ],
    ),
    (
        "provider_attempts",
        &[
            "provider_response_id",
            "response_model",
            "response_state",
            "provider_created_at",
            "incomplete_reason",
            "error_code",
            "transport_error",
            "observation_status",
            "streaming",
            "chunk_count",
            "byte_count",
            "ttfb_us",
            "ttft_us",
            "duration_us",
            "anomaly_metadata",
        ],
    ),
    (
        "provider_usage",
        &[
            "raw_usage_json",
            "input_cached",
            "output_reasoning",
            "total",
            "normalizer_version",
            "anomaly_metadata",
        ],
    ),
];

#[test]
fn released_v1_and_v2_migrations_are_byte_identical() {
    // Given: the released migrations, which no later phase may edit.
    let v1 = format!("{:x}", Sha256::digest(schema::MIGRATION_V1.as_bytes()));
    let v2 = format!("{:x}", Sha256::digest(schema::MIGRATION_V2.as_bytes()));

    // Then: their bytes still hash to the pinned digests.
    assert_eq!(
        v1, MIGRATION_V1_SHA256,
        "0001_initial.sql is immutable once released"
    );
    assert_eq!(
        v2, MIGRATION_V2_SHA256,
        "0002_provider_observability.sql is immutable once released"
    );
}

#[tokio::test]
async fn migration_from_empty_and_reopen() -> TestResult {
    // Given: an unused path inside a real temporary directory.
    let directory = TempDir::new()?;
    let database = directory.path().join("tracepress.sqlite3");
    let queue_items = MaxIpcQueueItems::new(4)?;

    // When: balanced startup migrates the file, then strict startup reopens it.
    let balanced = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Balanced,
        queue_items,
    ))
    .await?;
    assert_eq!(balanced.settings().journal_mode(), "wal");
    assert!(balanced.settings().foreign_keys());
    assert_eq!(balanced.settings().busy_timeout_ms(), 5_000);
    assert_eq!(balanced.settings().synchronous(), 1);
    balanced.shutdown().await?;

    let strict = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Strict,
        queue_items,
    ))
    .await?;
    assert_eq!(strict.settings().synchronous(), 2);
    strict.shutdown().await?;

    // Then: the one v1 record and every canonical table exist exactly once.
    let connection = Connection::open(database)?;
    let version: i64 = connection.query_row(
        "SELECT MAX(schema_version) FROM schema_metadata",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 3);
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(tables, TABLES);
    Ok(())
}

#[tokio::test]
async fn unsupported_schema_version_above_v3_is_rejected_on_open() -> TestResult {
    // Given: a real database whose schema metadata records a future version.
    let directory = TempDir::new()?;
    let database = directory.path().join("newer-schema.sqlite3");
    let connection = Connection::open(&database)?;
    connection.execute_batch(
        "CREATE TABLE schema_metadata (schema_version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL) STRICT;
         INSERT INTO schema_metadata(schema_version, applied_at) VALUES (4, '2026-09-08T00:00:00Z');",
    )?;
    drop(connection);
    let config = StorageConfig::new(database, Durability::Balanced, MaxIpcQueueItems::new(1)?);

    // When: the writer attempts to open the newer database.
    let result = StorageWriter::open(config).await;

    // Then: startup returns the stable typed version mismatch.
    assert!(matches!(
        result,
        Err(StorageError::UnsupportedSchemaVersion {
            found: 4,
            supported: 3
        })
    ));
    Ok(())
}

#[test]
fn sqlite_busy_and_interrupted_migration_leaves_prior_or_complete_state() -> TestResult {
    // Given: a fresh real database path and a migration failpoint after schema SQL.
    let directory = TempDir::new()?;
    let database = directory.path().join("interrupted.sqlite3");

    // When: migration is interrupted before its version record can commit.
    let result =
        schema::open_database_with_interrupted_migration(&database, Durability::Balanced, 1);

    // Then: the transaction leaves the prior empty state and a normal reopen migrates fully.
    assert!(matches!(result, Err(StorageError::InjectedFailure)));
    let connection = Connection::open(&database)?;
    let table_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(table_count, 0);
    drop(connection);
    let (connection, _settings) = schema::open_database(&database, Durability::Balanced)?;
    let version: i64 = connection.query_row(
        "SELECT MAX(schema_version) FROM schema_metadata",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 3);
    Ok(())
}

#[test]
fn interrupted_v2_migration_rolls_back_atomically_from_v1() -> TestResult {
    // Given: a real v1 database with its committed schema metadata.
    let directory = TempDir::new()?;
    let database = directory.path().join("interrupted-v2.sqlite3");
    let connection = Connection::open(&database)?;
    connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
    let rows_changed = connection.execute(
        "INSERT INTO schema_metadata(schema_version, applied_at) VALUES (1, '2026-09-09T00:00:00Z')",
        [],
    )?;
    assert_eq!(rows_changed, 1);
    drop(connection);

    // When: v2 migration is interrupted after its ALTER TABLE statements.
    let result =
        schema::open_database_with_interrupted_migration(&database, Durability::Balanced, 2);

    // Then: v2 fails, but the committed v1 metadata and schema remain intact.
    assert!(matches!(result, Err(StorageError::InjectedFailure)));
    let connection = Connection::open(&database)?;
    let mut statement =
        connection.prepare("SELECT schema_version FROM schema_metadata ORDER BY schema_version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(versions, vec![1]);
    for (table, columns) in V2_COLUMNS {
        let actual = table_columns(&connection, table)?;
        for column in columns {
            assert!(
                !actual.iter().any(|actual| actual.as_str() == *column),
                "v2 column {table}.{column} survived the interrupted migration"
            );
        }
    }
    drop(statement);
    drop(connection);

    // Finally: a normal reopen applies v2 and exposes every v2 column.
    let (connection, _settings) = schema::open_database(&database, Durability::Balanced)?;
    let mut statement =
        connection.prepare("SELECT schema_version FROM schema_metadata ORDER BY schema_version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(versions, vec![1, 2, 3]);
    for (table, columns) in V2_COLUMNS {
        let actual = table_columns(&connection, table)?;
        for column in columns {
            assert!(
                actual.iter().any(|actual| actual.as_str() == *column),
                "v2 column {table}.{column} was not applied on reopen"
            );
        }
    }
    Ok(())
}

#[test]
fn interrupted_v3_migration_rolls_back_atomically_from_v2() -> TestResult {
    // Given: a real v2 database with both committed schema metadata records.
    let directory = TempDir::new()?;
    let database = directory.path().join("interrupted-v3.sqlite3");
    let connection = Connection::open(&database)?;
    connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
    connection.execute_batch(include_str!(
        "../migrations/0002_provider_observability.sql"
    ))?;
    let rows_changed = connection.execute(
        "INSERT INTO schema_metadata(schema_version, applied_at) VALUES (1, '2026-09-09T00:00:00Z'), (2, '2026-09-09T00:00:01Z')",
        [],
    )?;
    assert_eq!(rows_changed, 2);
    drop(connection);

    // When: the v3 migration is interrupted after its CREATE TABLE statements.
    let result =
        schema::open_database_with_interrupted_migration(&database, Durability::Balanced, 3);

    // Then: v3 fails and v2 stays visible without a single v3 table or column.
    assert!(matches!(result, Err(StorageError::InjectedFailure)));
    let connection = Connection::open(&database)?;
    let mut statement =
        connection.prepare("SELECT schema_version FROM schema_metadata ORDER BY schema_version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(versions, vec![1, 2]);
    for table in V3_TABLES {
        assert!(
            table_columns(&connection, table)?.is_empty(),
            "v3 table {table} survived the interrupted migration"
        );
    }
    for (table, columns) in V2_COLUMNS {
        let actual = table_columns(&connection, table)?;
        for column in columns {
            assert!(
                actual.iter().any(|actual| actual.as_str() == *column),
                "v2 column {table}.{column} was lost by the interrupted v3 migration"
            );
        }
    }
    drop(statement);
    drop(connection);

    // Finally: a normal reopen applies v3 and creates every v3 table.
    let (connection, _settings) = schema::open_database(&database, Durability::Balanced)?;
    let mut statement =
        connection.prepare("SELECT schema_version FROM schema_metadata ORDER BY schema_version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(versions, vec![1, 2, 3]);
    for table in V3_TABLES {
        assert!(
            !table_columns(&connection, table)?.is_empty(),
            "v3 table {table} was not applied on reopen"
        );
    }
    Ok(())
}

#[tokio::test]
async fn existing_phase_two_database_upgrades_to_v3_without_losing_rows() -> TestResult {
    // Given: a populated v2 database written before Phase 3 existed.
    let directory = TempDir::new()?;
    let database = directory.path().join("phase-two.sqlite3");
    let connection = Connection::open(&database)?;
    connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
    connection.execute_batch(include_str!(
        "../migrations/0002_provider_observability.sql"
    ))?;
    connection.execute_batch(
        "INSERT INTO schema_metadata(schema_version, applied_at) VALUES (1, '2026-09-09T00:00:00Z'), (2, '2026-09-09T00:00:01Z');
         INSERT INTO sessions(session_id, started_at, ended_at, state, ingress_key) VALUES ('phase-two-session', '2026-09-09T00:00:02Z', NULL, 'active', 'phase-two-ingress');
         INSERT INTO operations(operation_id, session_id, kind, started_at, ended_at, status) VALUES ('phase-two-operation', 'phase-two-session', 'llm_inference', '2026-09-09T00:00:03Z', NULL, 'started');
         INSERT INTO provider_requests(request_id, operation_id, route, method, request_bytes, provider, protocol, model) VALUES ('phase-two-request', 'phase-two-operation', 'responses', 'post', 2048, 'openai', 'openai-responses-v1', 'gpt-5');
         INSERT INTO provider_attempts(attempt_id, request_id, ordinal, status_code, started_at, ended_at, status) VALUES ('phase-two-attempt', 'phase-two-request', 0, 200, '2026-09-09T00:00:04Z', NULL, 'completed');
         INSERT INTO provider_usage(attempt_id, input_total) VALUES ('phase-two-attempt', 1200);",
    )?;
    drop(connection);

    // When: the Phase 3 writer opens that existing database.
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Balanced,
        MaxIpcQueueItems::new(2)?,
    ))
    .await?;
    writer.shutdown().await?;

    // Then: v3 is recorded additively, the Phase 2 rows survive, and the new tables are empty.
    let connection = Connection::open(database)?;
    let mut statement =
        connection.prepare("SELECT schema_version FROM schema_metadata ORDER BY schema_version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(versions, vec![1, 2, 3]);
    let preserved: (String, i64, i64) = connection.query_row(
        "SELECT model, request_bytes, (SELECT input_total FROM provider_usage WHERE attempt_id = 'phase-two-attempt') FROM provider_requests WHERE request_id = 'phase-two-request'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(preserved, ("gpt-5".to_owned(), 2_048, 1_200));
    for table in V3_TABLES {
        let rows: i64 =
            connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        assert_eq!(
            rows, 0,
            "{table} must be created empty by an additive migration"
        );
    }
    Ok(())
}
