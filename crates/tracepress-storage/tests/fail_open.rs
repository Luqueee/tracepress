//! Named storage exhaustion and contention fail-open scenarios.

use rusqlite::Connection;
use tempfile::TempDir;
use tracepress_core::{ContentKind, MaxIpcQueueItems, SessionId, SessionState, UuidV7Generator};
use tracepress_storage::{
    BlobByteLimit, BlobError, BlobPut, BlobStore, BlobStoreConfig, Durability, HybridBlobStore,
    InlineBlobMaxBytes, StorageConfig, StorageError, StorageWriter, WriteCommand,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn disk_full_preserves_raw_input_before_persistence() -> TestResult {
    let directory = TempDir::new()?;
    let raw = b"raw bytes remain owned by caller";
    let store = open_store(&directory, 1).await?;
    let result = store
        .put(BlobPut::new(
            raw,
            ContentKind::Binary,
            "2026-09-09T00:00:00Z",
        ))
        .await;
    assert!(matches!(result, Err(BlobError::TooLarge { .. })));
    assert_eq!(raw.as_slice(), b"raw bytes remain owned by caller");
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cas_failure_creates_no_durable_reference() -> TestResult {
    let directory = TempDir::new()?;
    let store = open_store(&directory, 4096).await?;
    let objects = directory.path().join("cas/objects");
    std::fs::remove_dir(&objects)?;
    std::fs::write(&objects, b"block directory creation")?;
    let raw = b"forwardable original";
    let result = store
        .put(BlobPut::new(
            raw,
            ContentKind::Binary,
            "2026-09-09T00:00:00Z",
        ))
        .await;
    assert!(matches!(result, Err(BlobError::Filesystem { .. })));
    assert_eq!(raw.as_slice(), b"forwardable original");
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn sqlite_busy_returns_typed_error_without_partial_commit() -> TestResult {
    let directory = TempDir::new()?;
    let database = directory.path().join("busy.sqlite3");
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Balanced,
        MaxIpcQueueItems::new(1)?,
    ))
    .await?;
    let lock = Connection::open(&database)?;
    lock.execute_batch("BEGIN IMMEDIATE")?;
    let ids = UuidV7Generator::new();
    let result = writer
        .submit(WriteCommand::Session {
            session_id: SessionId::generate(&ids),
            started_at: "2026-09-09T00:00:00Z".to_owned(),
            ended_at: None,
            state: SessionState::Active,
            ingress_key: "busy".to_owned(),
        })
        .await;
    assert!(matches!(result, Err(StorageError::Busy(_))));
    lock.execute_batch("ROLLBACK")?;
    writer.shutdown().await?;
    let rows: i64 =
        Connection::open(database)?
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
    assert_eq!(rows, 0);
    Ok(())
}

async fn open_store(
    directory: &TempDir,
    persisted_limit: u64,
) -> Result<HybridBlobStore, BlobError> {
    let storage = StorageConfig::new(
        directory.path().join("blob.sqlite3"),
        Durability::Strict,
        MaxIpcQueueItems::new(8).map_err(|_| BlobError::ZeroLimit { field: "queue" })?,
    );
    let blobs = BlobStoreConfig::new(
        directory.path().join("cas"),
        InlineBlobMaxBytes::new(0)?,
        BlobByteLimit::new(4096)?,
    )
    .with_max_persisted_bytes(BlobByteLimit::new(persisted_limit)?);
    HybridBlobStore::open(storage, blobs).await
}
