use std::time::Duration;

use rusqlite::Connection;
use tempfile::TempDir;
use tracepress_core::{MaxIpcQueueItems, SessionId, SessionState, UuidV7Generator};

use crate::test_support::TestResult;
use crate::{Durability, StorageConfig, StorageError, StorageWriter, WriteCommand, WriteReceipt};

#[tokio::test]
async fn sqlite_busy_returns_typed_bounded_failure_without_partial_commit() -> TestResult {
    // Given: a writer with a short test-only bound and a second connection holding a write lock.
    let directory = TempDir::new()?;
    let database = directory.path().join("busy.sqlite3");
    let queue_items = MaxIpcQueueItems::new(1)?;
    let writer = StorageWriter::open(
        StorageConfig::new(database.clone(), Durability::Balanced, queue_items)
            .with_test_busy_timeout(Duration::from_millis(10)),
    )
    .await?;
    let lock = Connection::open(&database)?;
    lock.execute_batch("BEGIN IMMEDIATE")?;

    // When: the writer attempts a transaction while the independent lock is held.
    let result = writer.submit(session_command("busy-ingress")).await;

    // Then: the bounded result is typed and no session was partially inserted.
    assert!(matches!(result, Err(StorageError::Busy(_))));
    lock.execute_batch("ROLLBACK")?;
    writer.shutdown().await?;
    let connection = Connection::open(database)?;
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
    assert_eq!(count, 0);
    Ok(())
}

#[tokio::test]
async fn bounded_queue_backpressures_and_shutdown_drains_accepted_command() -> TestResult {
    // Given: a capacity-one writer paused by synchronization primitives inside its worker.
    let directory = TempDir::new()?;
    let database = directory.path().join("backpressure.sqlite3");
    let queue_items = MaxIpcQueueItems::new(1)?;
    let writer = StorageWriter::open(StorageConfig::new(
        database.clone(),
        Durability::Balanced,
        queue_items,
    ))
    .await?;
    let (entered, release) = writer.pause_for_test().await?;
    entered.await?;
    let accepted = writer
        .enqueue_for_test(session_command("accepted-ingress"))
        .await?;

    // When: another caller tries the full queue and shutdown begins before the pause releases.
    let result = writer.try_submit(session_command("rejected-ingress")).await;
    let shutdown = tokio::spawn(writer.shutdown());
    assert!(release.send(()).is_ok());

    // Then: the extra command is rejected, while accepted work commits before shutdown succeeds.
    assert!(matches!(result, Err(StorageError::QueueFull)));
    let receipt = accepted.await??;
    assert!(matches!(
        receipt,
        WriteReceipt::Committed { rows_changed: 1 }
    ));
    shutdown.await??;
    let connection = Connection::open(database)?;
    let ingress: String =
        connection.query_row("SELECT ingress_key FROM sessions", [], |row| row.get(0))?;
    assert_eq!(ingress, "accepted-ingress");
    Ok(())
}

fn session_command(ingress_key: &str) -> WriteCommand {
    WriteCommand::Session {
        session_id: SessionId::generate(&UuidV7Generator::new()),
        started_at: "2026-09-08T22:00:00Z".to_owned(),
        ended_at: None,
        state: SessionState::Active,
        ingress_key: ingress_key.to_owned(),
    }
}
