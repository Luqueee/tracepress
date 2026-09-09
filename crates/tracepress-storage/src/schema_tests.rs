use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use tracepress_core::MaxIpcQueueItems;

use crate::test_support::TestResult;
use crate::{Durability, StorageConfig, StorageError, StorageWriter, schema};

/// Digest of the released v1 migration, which the contract declares immutable.
const MIGRATION_V1_SHA256: &str =
    "4a08ae8e8bfd52f0eb427ccaed02b11bb1d6a12676b11efd77356b7e66acc119";

const TABLES: [&str; 15] = [
    "causal_edges",
    "compression_decisions",
    "content_bindings",
    "content_objects",
    "content_occurrences",
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
fn released_v1_migration_is_byte_identical_and_v2_declares_only_writable_columns() {
    // Given: the released v1 migration, which no later phase may edit.
    let digest = format!("{:x}", Sha256::digest(schema::MIGRATION_V1.as_bytes()));

    // Then: its bytes still hash to the pinned digest.
    assert_eq!(
        digest, MIGRATION_V1_SHA256,
        "0001_initial.sql is immutable once released"
    );

    // And: v2 declares no attempt timestamp column the writer can never populate.
    for column in [
        "completed_at",
        "first_upstream_byte_at",
        "first_semantic_output_at",
    ] {
        assert!(
            !schema::MIGRATION_V2.contains(column),
            "v2 must not declare {column}, which no producer can populate"
        );
    }
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
    assert_eq!(version, 2);
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
async fn unsupported_schema_version_above_v2_is_rejected_on_open() -> TestResult {
    // Given: a real database whose schema metadata records a future version.
    let directory = TempDir::new()?;
    let database = directory.path().join("newer-schema.sqlite3");
    let connection = Connection::open(&database)?;
    connection.execute_batch(
        "CREATE TABLE schema_metadata (schema_version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL) STRICT;
         INSERT INTO schema_metadata(schema_version, applied_at) VALUES (3, '2026-09-08T00:00:00Z');",
    )?;
    drop(connection);
    let config = StorageConfig::new(database, Durability::Balanced, MaxIpcQueueItems::new(1)?);

    // When: the writer attempts to open the newer database.
    let result = StorageWriter::open(config).await;

    // Then: startup returns the stable typed version mismatch.
    assert!(matches!(
        result,
        Err(StorageError::UnsupportedSchemaVersion {
            found: 3,
            supported: 2
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
    let result = schema::open_database_with_interrupted_migration(&database, Durability::Balanced);

    // Then: the transaction leaves the prior empty state and a normal reopen completes v1.
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
    assert_eq!(version, 2);
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
        schema::open_database_with_interrupted_v2_migration(&database, Durability::Balanced);

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
    assert_eq!(versions, vec![1, 2]);
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
