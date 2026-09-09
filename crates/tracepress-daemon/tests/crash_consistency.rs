//! Restart and stale-session durability checks.

use std::{path::Path, time::Duration};

use tempfile::TempDir;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracepress_core::{
    MaxIpcFrameBytes, MaxIpcQueueItems, MaxRequestBodyBytes, MaxResponseBodyBytes, OperationKind,
    RequestId, UuidV7Generator,
};
use tracepress_daemon::{ControlRequest, ControlResponse, DaemonService};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcLimits, IpcRequest, ResponseOutcome, UnixEndpoint,
};
use tracepress_storage::{Durability, StorageConfig, StorageWriter};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const CREDENTIAL: [u8; 32] = [0x5a; 32];

#[tokio::test]
async fn restart_after_interruption_marks_active_session_stale() -> TestResult {
    let directory = TempDir::new()?;
    let database = directory.path().join("tracepress.sqlite3");
    let first = open(&database).await?;
    let session = first.start_session("2026-09-09T00:00:00Z").await?;
    let operation = first
        .create_operation(
            session.session_id,
            OperationKind::Agent,
            "2026-09-09T00:00:01Z",
            None,
        )
        .await?;
    assert_eq!(first.session(session.session_id).await?.operations, 1);
    let _ = operation;
    first.shutdown().await?;

    let recovered = open(&database).await?;
    assert_eq!(recovered.recovered_stale_sessions(), 1);
    recovered.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn subprocess_kill_recovers_committed_session_without_dangling_operation() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = TempDir::new()?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    let database = directory.path().join("tracepress.sqlite3");
    let socket = directory.path().join("tracepress.sock");
    let credential = directory.path().join("control.cred");
    let ready = directory.path().join("ready");
    std::fs::write(&credential, "5a".repeat(32))?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_tracepressd"))
        .env("TRACEPRESS_DATABASE", &database)
        .env("TRACEPRESS_CONTROL_SOCKET", &socket)
        .env("TRACEPRESS_CONTROL_CREDENTIAL", &credential)
        .env("TRACEPRESS_DAEMON_READY", &ready)
        .spawn()?;
    wait_ready(&ready).await?;
    let response = control(
        &socket,
        ControlRequest::StartSession {
            started_at: "2026-09-09T00:00:00Z".to_owned(),
        },
    )
    .await?;
    let session = match response {
        ControlResponse::Ok {
            session: Some(session),
            ..
        } => session,
        other => return Err(format!("unexpected start response: {other:?}").into()),
    };
    assert_eq!(session.operations, 1);

    child.kill().await?;
    let _status = child.wait().await?;
    std::fs::remove_file(&ready)?;
    let mut restarted = Command::new(env!("CARGO_BIN_EXE_tracepressd"))
        .env("TRACEPRESS_DATABASE", &database)
        .env("TRACEPRESS_CONTROL_SOCKET", &socket)
        .env("TRACEPRESS_CONTROL_CREDENTIAL", &credential)
        .env("TRACEPRESS_DAEMON_READY", &ready)
        .spawn()?;
    wait_ready(&ready).await?;
    let shutdown = control(&socket, ControlRequest::Shutdown).await?;
    assert!(matches!(shutdown, ControlResponse::Ok { .. }));
    let status = restarted.wait().await?;
    assert!(status.success());

    let connection = rusqlite::Connection::open(database)?;
    let state: String = connection.query_row("SELECT state FROM sessions", [], |row| row.get(0))?;
    let operations: i64 =
        connection.query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))?;
    let dangling: i64 = connection.query_row(
        "SELECT COUNT(*) FROM operations o LEFT JOIN sessions s ON s.session_id = o.session_id WHERE s.session_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(state, "stale");
    assert_eq!(operations, 1);
    assert_eq!(dangling, 0);
    Ok(())
}

async fn control(
    socket: &Path,
    request: ControlRequest,
) -> Result<ControlResponse, Box<dyn std::error::Error>> {
    let limits = IpcLimits::new(
        MaxIpcFrameBytes::new(65_536)?,
        MaxRequestBodyBytes::new(32_768)?,
        MaxResponseBodyBytes::new(32_768)?,
    );
    let cancellation = CancellationToken::new();
    let client = IpcClient::authenticated(
        Endpoint::Unix(UnixEndpoint::new(socket.to_path_buf())?),
        Credential::new(CREDENTIAL),
        limits,
    );
    let mut connection = client.connect(&cancellation).await?;
    let request = IpcRequest::new(
        RequestId::generate(&UuidV7Generator::new()),
        serde_json::to_vec(&request)?,
        limits.maximum_request_body(),
    )?;
    connection.send_request(&request, &cancellation).await?;
    let response = connection.receive_response(&cancellation).await?;
    match response.outcome() {
        ResponseOutcome::Complete { body } => Ok(serde_json::from_slice(body)?),
        ResponseOutcome::Incomplete | ResponseOutcome::Cancelled => {
            Err("daemon returned an incomplete response".into())
        }
    }
}

async fn wait_ready(path: &Path) -> TestResult {
    for _ in 0..100 {
        if path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err("daemon readiness timed out".into())
}

async fn open(path: &Path) -> Result<DaemonService, Box<dyn std::error::Error>> {
    let writer = StorageWriter::open(StorageConfig::new(
        path.to_path_buf(),
        Durability::Strict,
        MaxIpcQueueItems::new(4)?,
    ))
    .await?;
    Ok(DaemonService::open(writer, "2026-09-09T00:00:00Z").await?)
}
