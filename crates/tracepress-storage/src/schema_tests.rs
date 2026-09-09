use rusqlite::Connection;
use tempfile::TempDir;
use tracepress_core::MaxIpcQueueItems;

use crate::test_support::TestResult;
use crate::{Durability, StorageConfig, StorageError, StorageWriter, schema};

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
    assert_eq!(version, 1);
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
async fn unsupported_schema_version_above_v1_is_rejected_on_open() -> TestResult {
    // Given: a real database whose schema metadata records version 2.
    let directory = TempDir::new()?;
    let database = directory.path().join("newer-schema.sqlite3");
    let connection = Connection::open(&database)?;
    connection.execute_batch(
        "CREATE TABLE schema_metadata (schema_version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL) STRICT;
         INSERT INTO schema_metadata(schema_version, applied_at) VALUES (2, '2026-09-08T00:00:00Z');",
    )?;
    drop(connection);
    let config = StorageConfig::new(database, Durability::Balanced, MaxIpcQueueItems::new(1)?);

    // When: the v1 writer attempts to open the newer database.
    let result = StorageWriter::open(config).await;

    // Then: startup returns the stable typed version mismatch.
    assert!(matches!(
        result,
        Err(StorageError::UnsupportedSchemaVersion {
            found: 2,
            supported: 1
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
    assert_eq!(version, 1);
    Ok(())
}
