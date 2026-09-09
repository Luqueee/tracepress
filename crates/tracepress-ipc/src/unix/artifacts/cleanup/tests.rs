use std::{fs, sync::mpsc};

use super::{ArtifactKind, RemovalTarget, remove_matching_with_hook};
use crate::{IpcError, unix::artifacts::FileIdentity};

#[test]
fn cleanup_preserves_replacement_after_quarantine_verification()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("ipc.sock");
    let owned_listener = std::os::unix::net::UnixListener::bind(&socket)?;
    let expected = FileIdentity::from_metadata(&fs::symlink_metadata(&socket)?);
    let (path_sender, path_receiver) = mpsc::sync_channel(0);
    let (ready_sender, ready_receiver) = mpsc::sync_channel(0);
    let replacer = std::thread::spawn(move || {
        let quarantine = path_receiver.recv().map_err(std::io::Error::other)?;
        fs::remove_file(&quarantine)?;
        let listener = std::os::unix::net::UnixListener::bind(&quarantine)?;
        ready_sender.send(()).map_err(std::io::Error::other)?;
        Ok::<_, std::io::Error>(listener)
    });

    // When
    let target = RemovalTarget::new(&socket, expected, ArtifactKind::Socket);
    let result = remove_matching_with_hook(target, |path| {
        path_sender
            .send(path.to_path_buf())
            .map_err(std::io::Error::other)?;
        ready_receiver.recv().map_err(std::io::Error::other)?;
        Ok(())
    });

    // Then
    let preserved = std::os::unix::net::UnixStream::connect(&socket).is_ok();
    let changed = matches!(
        result,
        Err(IpcError::SocketArtifactChanged {
            artifact: "socket",
            ..
        })
    );
    drop(owned_listener);
    let listener = replacer
        .join()
        .map_err(|_| std::io::Error::other("replacement thread panicked"))??;
    drop(listener);
    if socket.exists() {
        fs::remove_file(socket)?;
    }
    assert!(preserved);
    assert!(changed);
    Ok(())
}
