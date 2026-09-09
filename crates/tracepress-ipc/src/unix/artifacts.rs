use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{ErrorKind, Read as _, Write as _},
    os::unix::fs::{FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use subtle::ConstantTimeEq;

use crate::{IpcError, endpoint::SOCKET_OWNER_BYTES};

mod cleanup;
mod private_directory;
mod publication;

use self::cleanup::{ArtifactKind, remove_matching, verify_identity};
pub(super) use self::publication::StagedSocket;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    pub(super) fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Debug)]
pub(super) struct SocketArtifacts {
    socket: PathBuf,
    marker: PathBuf,
    marker_identity: Option<FileIdentity>,
    socket_identity: Option<FileIdentity>,
}

impl SocketArtifacts {
    pub(super) const fn new(socket: PathBuf, marker: PathBuf) -> Self {
        Self {
            socket,
            marker,
            marker_identity: None,
            socket_identity: None,
        }
    }

    pub(super) const fn set_marker(&mut self, identity: FileIdentity) {
        self.marker_identity = Some(identity);
    }

    pub(super) fn marker(&self) -> &Path {
        &self.marker
    }

    pub(super) const fn set_socket(&mut self, identity: FileIdentity) {
        self.socket_identity = Some(identity);
    }

    pub(super) fn cleanup(&mut self) -> Result<(), IpcError> {
        match (self.socket_identity, self.marker_identity) {
            (Some(socket), Some(marker)) => {
                verify_identity(&self.socket, socket, ArtifactKind::Socket)?;
                verify_identity(&self.marker, marker, ArtifactKind::Marker)?;
                remove_matching(&self.socket, socket, ArtifactKind::Socket)?;
                self.socket_identity = None;
                remove_matching(&self.marker, marker, ArtifactKind::Marker)?;
                self.marker_identity = None;
                Ok(())
            }
            (None, Some(marker)) => {
                remove_matching(&self.marker, marker, ArtifactKind::Marker)?;
                self.marker_identity = None;
                Ok(())
            }
            (Some(_) | None, None) => Ok(()),
        }
    }
}

impl Drop for SocketArtifacts {
    fn drop(&mut self) {
        if self.cleanup().is_err() {}
    }
}

pub(super) fn marker_path(socket: &Path) -> PathBuf {
    let mut marker = socket.as_os_str().to_os_string();
    marker.push(".owner");
    PathBuf::from(marker)
}

pub(super) fn prepare_socket_path(
    socket: &Path,
    marker: &Path,
    owner: &[u8; SOCKET_OWNER_BYTES],
) -> Result<(), IpcError> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) if !metadata.file_type().is_socket() => Err(IpcError::SocketPathNotSocket),
        Ok(metadata) => {
            let socket_identity = FileIdentity::from_metadata(&metadata);
            let marker_identity = require_owner(marker, owner)?;
            match std::os::unix::net::UnixStream::connect(socket) {
                Ok(_active) => Err(IpcError::SocketAlreadyActive),
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::ConnectionRefused | ErrorKind::NotFound
                    ) =>
                {
                    remove_matching(socket, socket_identity, ArtifactKind::Socket)?;
                    remove_matching(marker, marker_identity, ArtifactKind::Marker)
                }
                Err(_error) => Err(IpcError::SocketStateUnknown),
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            match fs::symlink_metadata(marker) {
                Ok(_metadata) => {
                    let marker_identity = require_owner(marker, owner)?;
                    remove_matching(marker, marker_identity, ArtifactKind::Marker)?;
                }
                Err(marker_error) if marker_error.kind() == ErrorKind::NotFound => {}
                Err(marker_error) => return Err(IpcError::from(marker_error)),
            }
            Ok(())
        }
        Err(error) => Err(IpcError::from(error)),
    }
}

pub(super) fn create_marker(marker: &Path) -> Result<(File, FileIdentity), IpcError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(marker)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(IpcError::SocketMarkerInvalid);
    }
    Ok((file, FileIdentity::from_metadata(&metadata)))
}

pub(super) fn write_marker(
    file: &mut File,
    owner: &[u8; SOCKET_OWNER_BYTES],
) -> Result<(), IpcError> {
    file.write_all(owner)?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn socket_identity(socket: &Path) -> Result<FileIdentity, IpcError> {
    let metadata = fs::symlink_metadata(socket)?;
    if !metadata.file_type().is_socket() {
        return Err(IpcError::SocketPathNotSocket);
    }
    Ok(FileIdentity::from_metadata(&metadata))
}

fn require_owner(
    marker: &Path,
    expected: &[u8; SOCKET_OWNER_BYTES],
) -> Result<FileIdentity, IpcError> {
    let (actual, identity) = read_marker(marker)?;
    if bool::from(actual.ct_eq(expected)) {
        Ok(identity)
    } else {
        Err(IpcError::SocketNotOwned)
    }
}

fn read_marker(marker: &Path) -> Result<([u8; SOCKET_OWNER_BYTES], FileIdentity), IpcError> {
    let before = fs::symlink_metadata(marker)?;
    if !before.file_type().is_file() {
        return Err(IpcError::SocketMarkerInvalid);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(marker)
        .map_err(|_error| IpcError::SocketMarkerInvalid)?;
    let opened = file.metadata()?;
    let identity = FileIdentity::from_metadata(&opened);
    if !opened.file_type().is_file()
        || identity != FileIdentity::from_metadata(&before)
        || opened.len()
            != u64::try_from(SOCKET_OWNER_BYTES)
                .map_err(|_error| IpcError::FrameLengthUnrepresentable)?
    {
        return Err(IpcError::SocketMarkerInvalid);
    }
    let mut owner = [0_u8; SOCKET_OWNER_BYTES];
    file.read_exact(&mut owner)?;
    Ok((owner, identity))
}
