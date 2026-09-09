//! Cross-process Unix client behavior through the public connector API.
#![cfg(unix)]

use std::{
    fs,
    io::{BufRead as _, Read as _, Write as _},
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use tokio_util::sync::CancellationToken;
use tracepress_core::{
    MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes, RequestId, UuidV7Generator,
};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcLimits, IpcRequest, IpcResponse, IpcTransport,
    ResponseOutcome, SocketOwner, UnixBinding, UnixEndpoint, UnixTransport, UnixTransportConfig,
};
use zeroize::Zeroizing;

const SOCKET_ENV: &str = "TRACEPRESS_IPC_TEST_SOCKET";

fn limits() -> Result<IpcLimits, tracepress_core::LimitValueError> {
    Ok(IpcLimits::new(
        MaxIpcFrameBytes::new(4_096)?,
        MaxRequestBodyBytes::new(64)?,
        MaxResponseBodyBytes::new(64)?,
    ))
}

fn binding(path: &Path) -> Result<UnixBinding, tracepress_ipc::IpcError> {
    Ok(UnixBinding::new(
        UnixEndpoint::new(path.to_path_buf())?,
        SocketOwner::new([61; 16]),
    ))
}

#[tokio::test]
async fn independent_subprocess_connects_without_listener_ownership()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = directory.path().join("ipc.sock");
    let credential_bytes = Zeroizing::new([62_u8; 32]);
    let credential = Credential::new(*credential_bytes);
    let transport = UnixTransport::bind(UnixTransportConfig::authenticated(
        binding(&socket)?,
        credential.clone(),
        limits()?,
    ))?;
    let mut child = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "independent_subprocess_client_helper",
            "--ignored",
            "--nocapture",
        ])
        .env(SOCKET_ENV, &socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("child stdin was not piped")?
        .write_all(&credential_bytes[..])?;
    let _child_stdout = wait_until_ready(&mut child)?;
    let cancellation = CancellationToken::new();

    // When
    let mut connection = transport.accept(&cancellation).await?;
    let request = connection.receive_request(&cancellation).await?;
    connection
        .send_response(
            &IpcResponse::new(
                request.request_id(),
                ResponseOutcome::complete(request.body().to_vec(), MaxResponseBodyBytes::new(64)?)?,
            ),
            &cancellation,
        )
        .await?;
    let status = child.wait()?;

    // Then
    assert!(status.success());
    assert_eq!(request.body(), [63, 0]);
    transport.close()?;
    Ok(())
}

#[tokio::test]
#[ignore = "invoked as a subprocess by the parent regression"]
async fn independent_subprocess_client_helper() -> Result<(), Box<dyn std::error::Error>> {
    let socket =
        PathBuf::from(std::env::var_os(SOCKET_ENV).ok_or("socket path was not inherited")?);
    let mut credential_bytes = Zeroizing::new([0_u8; 32]);
    std::io::stdin().read_exact(&mut credential_bytes[..])?;
    let client = IpcClient::authenticated(
        Endpoint::Unix(UnixEndpoint::new(socket)?),
        Credential::new(*credential_bytes),
        limits()?,
    );
    std::io::stdout().write_all(b"READY\n")?;
    std::io::stdout().flush()?;
    let cancellation = CancellationToken::new();
    let request = IpcRequest::new(
        RequestId::generate(&UuidV7Generator::new()),
        vec![63, 0],
        MaxRequestBodyBytes::new(64)?,
    )?;
    let mut connection = client.connect(&cancellation).await?;
    connection.send_request(&request, &cancellation).await?;
    let response = connection.receive_response(&cancellation).await?;
    assert_eq!(response.request_id(), request.request_id());
    assert_eq!(
        response.outcome(),
        &ResponseOutcome::complete(vec![63, 0], MaxResponseBodyBytes::new(64)?)?
    );
    Ok(())
}

fn wait_until_ready(
    child: &mut std::process::Child,
) -> Result<std::io::BufReader<std::process::ChildStdout>, Box<dyn std::error::Error>> {
    let stdout = child.stdout.take().ok_or("child stdout was not piped")?;
    let mut reader = std::io::BufReader::new(stdout);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err("child exited before readiness signal".into());
        }
        if line.contains("READY") {
            return Ok(reader);
        }
    }
}
