//! Public daemon lifecycle and causal-DAG integration tests.

use tempfile::TempDir;
use tracepress_core::{CausalRelationship, MaxIpcQueueItems, OperationKind, SessionState};
use tracepress_daemon::{DaemonError, DaemonService};
use tracepress_storage::{Durability, StorageConfig, StorageWriter};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn session_and_dag_lifecycle_supports_parallel_branches() -> TestResult {
    // Given: one daemon-owned writer and an isolated active session.
    let directory = TempDir::new()?;
    let daemon = daemon(&directory, 8).await?;
    let session = daemon.start_session("2026-09-09T00:00:00Z").await?;

    // When: one root operation spawns two independent children.
    let root = daemon
        .create_operation(
            session.session_id,
            OperationKind::Agent,
            "2026-09-09T00:00:01Z",
            None,
        )
        .await?;
    let first = daemon
        .create_operation(
            session.session_id,
            OperationKind::ToolExecution,
            "2026-09-09T00:00:02Z",
            Some((root, CausalRelationship::Spawned)),
        )
        .await?;
    let second = daemon
        .create_operation(
            session.session_id,
            OperationKind::LlmInference,
            "2026-09-09T00:00:03Z",
            Some((root, CausalRelationship::Spawned)),
        )
        .await?;

    // Then: identities differ and the public snapshot contains every DAG node.
    assert_ne!(first, second);
    assert_ne!(root, first);
    assert_eq!(daemon.session(session.session_id).await?.operations, 3);
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn shutdown_and_stale_session_rejects_new_operations() -> TestResult {
    // Given: an active session with one committed root operation.
    let directory = TempDir::new()?;
    let daemon = daemon(&directory, 1).await?;
    let session = daemon.start_session("2026-09-09T00:00:00Z").await?;
    let _root = daemon
        .create_operation(
            session.session_id,
            OperationKind::Agent,
            "2026-09-09T00:00:01Z",
            None,
        )
        .await?;

    // When: restart recovery marks the session stale before another operation is requested.
    let stale = daemon
        .finish_session(
            session.session_id,
            SessionState::Stale,
            "2026-09-09T00:00:02Z",
        )
        .await?;
    let result = daemon
        .create_operation(
            session.session_id,
            OperationKind::ToolExecution,
            "2026-09-09T00:00:03Z",
            None,
        )
        .await;

    // Then: stale is observable and cannot be mistaken for an active session.
    assert_eq!(stale.state, SessionState::Stale);
    assert!(matches!(
        result,
        Err(DaemonError::SessionNotActive {
            state: SessionState::Stale,
            ..
        })
    ));
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn startup_recovery_marks_interrupted_active_sessions_stale() -> TestResult {
    // Given: a prior daemon exits while one durable session remains active.
    let directory = TempDir::new()?;
    let first = daemon(&directory, 4).await?;
    let _session = first.start_session("2026-09-09T00:00:00Z").await?;
    first.shutdown().await?;

    // When: a new daemon instance opens the same database.
    let recovered = daemon(&directory, 4).await?;

    // Then: startup reports the exact interrupted-session count.
    assert_eq!(recovered.recovered_stale_sessions(), 1);
    recovered.shutdown().await?;
    Ok(())
}

async fn daemon(
    directory: &TempDir,
    queue_items: u64,
) -> Result<DaemonService, Box<dyn std::error::Error>> {
    let writer = StorageWriter::open(StorageConfig::new(
        directory.path().join("tracepress.sqlite3"),
        Durability::Strict,
        MaxIpcQueueItems::new(queue_items)?,
    ))
    .await?;
    Ok(DaemonService::open(writer, "2026-09-09T00:00:00Z").await?)
}
