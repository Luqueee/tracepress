//! Unix artifact replacement and no-follow cleanup regressions.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
    sync::mpsc,
};

use tracepress_core::{MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes};
use tracepress_ipc::{
    Credential, IpcError, IpcLimits, SocketOwner, UnixBinding, UnixEndpoint, UnixTransport,
    UnixTransportConfig,
};

fn config(path: &Path, owner: [u8; 16]) -> Result<UnixTransportConfig, Box<dyn std::error::Error>> {
    Ok(UnixTransportConfig::authenticated(
        UnixBinding::new(
            UnixEndpoint::new(path.to_path_buf())?,
            SocketOwner::new(owner),
        ),
        Credential::new([41; 32]),
        IpcLimits::new(
            MaxIpcFrameBytes::new(4_096)?,
            MaxRequestBodyBytes::new(1_024)?,
            MaxResponseBodyBytes::new(1_024)?,
        ),
    ))
}

#[tokio::test]
async fn cleanup_reports_and_preserves_socket_replaced_by_a_foreign_listener()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = directory.path().join("ipc.sock");
    let transport = UnixTransport::bind(config(&socket, [42; 16])?)?;
    let (start_sender, start_receiver) = mpsc::sync_channel(0);
    let (ready_sender, ready_receiver) = mpsc::sync_channel(0);
    let replacement_path = socket.clone();
    let replacer = std::thread::spawn(move || {
        start_receiver.recv().map_err(std::io::Error::other)?;
        fs::remove_file(&replacement_path)?;
        let listener = std::os::unix::net::UnixListener::bind(&replacement_path)?;
        ready_sender.send(()).map_err(std::io::Error::other)?;
        Ok::<_, std::io::Error>(listener)
    });
    start_sender.send(())?;
    ready_receiver.recv()?;

    // When
    let cleanup = transport.close();

    // Then
    assert!(socket.exists());
    let connected = std::os::unix::net::UnixStream::connect(&socket);
    assert!(connected.is_ok());
    let Err(IpcError::SocketArtifactChanged {
        artifact,
        expected_device,
        expected_inode,
        actual_device,
        actual_inode,
    }) = cleanup
    else {
        return Err("socket replacement lacked identity evidence".into());
    };
    assert_eq!(artifact, "socket");
    assert_eq!(expected_device, actual_device);
    assert_ne!(expected_inode, actual_inode);
    let Ok(listener_result) = replacer.join() else {
        return Err("replacement thread panicked".into());
    };
    drop(listener_result?);
    fs::remove_file(socket)?;
    Ok(())
}

#[tokio::test]
async fn bind_rejects_owner_marker_symlink_without_following_it()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = directory.path().join("ipc.sock");
    let marker = directory.path().join("ipc.sock.owner");
    let target = directory.path().join("foreign-owner");
    fs::write(&target, [43; 16])?;
    symlink(&target, &marker)?;

    // When
    let result = UnixTransport::bind(config(&socket, [43; 16])?);

    // Then
    assert!(matches!(result, Err(IpcError::SocketMarkerInvalid)));
    assert_eq!(fs::read(target)?, [43; 16]);
    assert!(fs::symlink_metadata(marker)?.file_type().is_symlink());
    Ok(())
}

#[tokio::test]
async fn bind_failure_removes_only_the_marker_created_by_this_attempt()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = directory.path().join("s".repeat(120));
    let marker = Path::new(&format!("{}.owner", socket.display())).to_path_buf();

    // When
    let result = UnixTransport::bind(config(&socket, [44; 16])?);

    // Then
    assert!(matches!(result, Err(IpcError::Io { .. })));
    assert!(!marker.exists());
    assert!(!socket.exists());
    Ok(())
}

#[tokio::test]
async fn cleanup_preserves_socket_when_owner_marker_is_replaced_by_a_symlink()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = directory.path().join("ipc.sock");
    let marker = directory.path().join("ipc.sock.owner");
    let target = directory.path().join("foreign-owner");
    let transport = UnixTransport::bind(config(&socket, [45; 16])?)?;
    fs::write(&target, [45; 16])?;
    fs::remove_file(&marker)?;
    symlink(&target, &marker)?;

    // When
    let cleanup = transport.close();

    // Then
    assert!(socket.exists());
    assert!(fs::symlink_metadata(&marker)?.file_type().is_symlink());
    assert_eq!(fs::read(&target)?, [45; 16]);
    assert!(matches!(
        cleanup,
        Err(IpcError::SocketArtifactChanged {
            artifact: "owner marker",
            ..
        })
    ));
    fs::remove_file(socket)?;
    fs::remove_file(marker)?;
    Ok(())
}
