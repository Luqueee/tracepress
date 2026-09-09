//! Private Unix socket lifecycle, ownership, and authentication behavior.
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt as _, path::Path};

use tokio_util::sync::CancellationToken;
use tracepress_core::{
    MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes, RequestId, UuidV7Generator,
};
use tracepress_ipc::{
    Credential, IpcClient, IpcError, IpcLimits, IpcRequest, IpcResponse, IpcTransport,
    ResponseOutcome, SocketOwner, UnixBinding, UnixEndpoint, UnixTransport, UnixTransportConfig,
};

const fn frame_limit() -> Result<MaxIpcFrameBytes, tracepress_core::LimitValueError> {
    MaxIpcFrameBytes::new(4_096)
}

fn limits() -> Result<IpcLimits, tracepress_core::LimitValueError> {
    Ok(IpcLimits::new(
        frame_limit()?,
        MaxRequestBodyBytes::new(1_024)?,
        MaxResponseBodyBytes::new(1_024)?,
    ))
}

fn endpoint(path: &Path) -> Result<UnixEndpoint, Box<dyn std::error::Error>> {
    Ok(UnixEndpoint::new(path.to_path_buf())?)
}

fn config(path: &Path, owner: [u8; 16]) -> Result<UnixTransportConfig, Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "socket has no parent")
    })?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    Ok(UnixTransportConfig::authenticated(
        UnixBinding::new(endpoint(path)?, SocketOwner::new(owner)),
        Credential::new([3; 32]),
        limits()?,
    ))
}

#[tokio::test]
async fn unix_authenticated_round_trip_preserves_nul_and_cleans_artifacts()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("ipc.sock");
    let transport = UnixTransport::bind(config(&socket, [1; 16])?)?;
    let socket_mode = fs::metadata(&socket)?.permissions().mode() & 0o777;
    assert_eq!(socket_mode, 0o600);
    let cancellation = CancellationToken::new();
    let request_id = RequestId::generate(&UuidV7Generator::new());
    let request = IpcRequest::new(request_id, vec![0, 6, 0], MaxRequestBodyBytes::new(3)?)?;
    let response_limit = MaxResponseBodyBytes::new(3)?;
    let client =
        IpcClient::authenticated(transport.endpoint(), Credential::new([3; 32]), limits()?);

    // When
    let server = async {
        let mut peer = transport.accept(&cancellation).await?;
        let received = peer.receive_request(&cancellation).await?;
        peer.send_response(
            &IpcResponse::new(
                received.request_id(),
                ResponseOutcome::complete(received.body().to_vec(), response_limit)?,
            ),
            &cancellation,
        )
        .await
    };
    let client = async {
        let mut peer = client.connect(&cancellation).await?;
        peer.send_request(&request, &cancellation).await?;
        peer.receive_response(&cancellation).await
    };
    let (server_result, client_result) = tokio::join!(server, client);
    server_result?;
    let response = client_result?;
    drop(transport);

    // Then
    assert_eq!(
        response.outcome(),
        &ResponseOutcome::complete(vec![0, 6, 0], MaxResponseBodyBytes::new(3)?)?
    );
    assert!(!socket.exists());
    assert!(!directory.path().join("ipc.sock.owner").exists());
    Ok(())
}

#[test]
fn unix_bind_refuses_non_socket_path_without_removing_it() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("ipc.sock");
    fs::write(&socket, b"not a socket")?;

    // When
    let result = UnixTransport::bind(config(&socket, [2; 16])?);
    let Err(error) = result else {
        return Err("non-socket path was removed or accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::SocketPathNotSocket));
    assert_eq!(fs::read(&socket)?, b"not a socket");
    Ok(())
}

#[test]
fn unix_bind_refuses_socket_owned_by_another_instance() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("ipc.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    fs::write(directory.path().join("ipc.sock.owner"), [5; 16])?;

    // When
    let result = UnixTransport::bind(config(&socket, [4; 16])?);
    let Err(error) = result else {
        return Err("foreign socket was removed or accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::SocketNotOwned));
    assert!(socket.exists());
    drop(listener);
    Ok(())
}

#[tokio::test]
async fn unix_bind_replaces_same_owner_stale_socket() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("ipc.sock");
    let stale = std::os::unix::net::UnixListener::bind(&socket)?;
    drop(stale);
    fs::write(directory.path().join("ipc.sock.owner"), [6; 16])?;

    // When
    let transport = UnixTransport::bind(config(&socket, [6; 16])?)?;

    // Then
    assert!(socket.exists());
    drop(transport);
    Ok(())
}

#[test]
fn unix_bind_refuses_same_owner_socket_while_it_is_active() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("ipc.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    fs::write(directory.path().join("ipc.sock.owner"), [9; 16])?;

    // When
    let result = UnixTransport::bind(config(&socket, [9; 16])?);
    let Err(error) = result else {
        return Err("active same-owner socket was replaced".into());
    };

    // Then
    assert!(matches!(error, IpcError::SocketAlreadyActive));
    drop(listener);
    Ok(())
}

#[test]
fn unix_bind_refuses_group_accessible_directory() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o750))?;
    let socket = directory.path().join("ipc.sock");
    let transport_config = UnixTransportConfig::authenticated(
        UnixBinding::new(endpoint(&socket)?, SocketOwner::new([10; 16])),
        Credential::new([3; 32]),
        limits()?,
    );

    // When
    let result = UnixTransport::bind(transport_config);
    let Err(error) = result else {
        return Err("insecure directory was accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::InsecureSocketDirectory));
    Ok(())
}

#[test]
fn unix_endpoint_rejects_nul_before_operating_system_calls()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let invalid = std::path::PathBuf::from("bad\0socket");

    // When
    let result = UnixEndpoint::new(invalid);
    let Err(error) = result else {
        return Err("NUL endpoint was accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::EndpointContainsNul));
    Ok(())
}

#[tokio::test]
async fn unix_accept_cancellation_is_typed_and_drop_removes_socket()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("ipc.sock");
    let transport = UnixTransport::bind(config(&socket, [8; 16])?)?;
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    // When
    let result = transport.accept(&cancellation).await;
    let Err(error) = result else {
        return Err("cancelled accept completed".into());
    };
    drop(transport);

    // Then
    assert!(matches!(error, IpcError::Cancelled));
    assert!(!socket.exists());
    Ok(())
}
