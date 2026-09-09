use std::{fs, os::unix::fs::PermissionsExt as _, path::Path, sync::mpsc};

use tracepress_core::{MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes};

use super::{UnixBinding, UnixTransport, UnixTransportConfig};
use crate::{Credential, IpcError, IpcLimits, SocketOwner, UnixEndpoint};

fn config(path: &Path) -> Result<UnixTransportConfig, Box<dyn std::error::Error>> {
    Ok(UnixTransportConfig::authenticated(
        UnixBinding::new(
            UnixEndpoint::new(path.to_path_buf())?,
            SocketOwner::new([71; 16]),
        ),
        Credential::new([72; 32]),
        IpcLimits::new(
            MaxIpcFrameBytes::new(4_096)?,
            MaxRequestBodyBytes::new(1_024)?,
            MaxResponseBodyBytes::new(1_024)?,
        ),
    ))
}

#[tokio::test]
async fn bind_rejects_public_replacement_before_ownership_recording()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = directory.path().join("ipc.sock");
    let replacement = socket.clone();
    let (replace_sender, replace_receiver) = mpsc::sync_channel(0);
    let (ready_sender, ready_receiver) = mpsc::sync_channel(0);
    let replacer = std::thread::spawn(move || {
        replace_receiver.recv().map_err(std::io::Error::other)?;
        match fs::remove_file(&replacement) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = std::os::unix::net::UnixListener::bind(&replacement)?;
        ready_sender.send(()).map_err(std::io::Error::other)?;
        Ok::<_, std::io::Error>(listener)
    });

    // When
    let result = UnixTransport::bind_with_hook(config(&socket)?, |_| {
        replace_sender.send(()).map_err(std::io::Error::other)?;
        ready_receiver.recv().map_err(std::io::Error::other)?;
        Ok(())
    });

    // Then
    let rejected = matches!(
        result,
        Err(IpcError::SocketArtifactChanged {
            artifact: "socket publication",
            ..
        })
    );
    assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
    drop(result);
    let listener = replacer
        .join()
        .map_err(|_| std::io::Error::other("replacement thread panicked"))??;
    drop(listener);
    match fs::remove_file(&socket) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    assert!(rejected);
    Ok(())
}
