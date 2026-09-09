//! Exact failure-path acceptance scenario for local IPC boundaries.
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt as _};

use tokio::io::{AsyncWriteExt as _, duplex};
use tokio_util::sync::CancellationToken;
use tracepress_core::{MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes};
use tracepress_ipc::{
    Credential, FrameConfig, IpcError, IpcLimits, IpcRequest, SocketOwner, UnixBinding,
    UnixEndpoint, UnixTransport, UnixTransportConfig, read_frame, read_message,
};

#[tokio::test]
async fn rejects_malformed_oversized_and_stale_state() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let maximum = MaxIpcFrameBytes::new(8)?;
    let cancellation = CancellationToken::new();
    let frame_config = FrameConfig::new(maximum, &cancellation);

    // When
    let (mut malformed_writer, mut malformed_reader) = duplex(16);
    malformed_writer.write_all(&1_u32.to_be_bytes()).await?;
    malformed_writer.write_all(b"{").await?;
    let malformed = read_message::<_, IpcRequest>(&mut malformed_reader, frame_config).await;

    let (mut oversized_writer, mut oversized_reader) = duplex(16);
    oversized_writer.write_all(&9_u32.to_be_bytes()).await?;
    let oversized = read_frame(&mut oversized_reader, frame_config).await;

    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = directory.path().join("ipc.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    fs::write(directory.path().join("ipc.sock.owner"), [31; 16])?;
    let endpoint = UnixEndpoint::new(socket.clone())?;
    let stale = UnixTransport::bind(UnixTransportConfig::authenticated(
        UnixBinding::new(endpoint, SocketOwner::new([32; 16])),
        Credential::new([33; 32]),
        IpcLimits::new(
            maximum,
            MaxRequestBodyBytes::new(8)?,
            MaxResponseBodyBytes::new(8)?,
        ),
    ));

    // Then
    assert!(matches!(malformed, Err(IpcError::MalformedMessage { .. })));
    assert!(matches!(
        oversized,
        Err(IpcError::FrameTooLarge {
            declared: 9,
            maximum: 8
        })
    ));
    assert!(matches!(stale, Err(IpcError::SocketNotOwned)));
    assert!(socket.exists());
    drop(listener);
    Ok(())
}
