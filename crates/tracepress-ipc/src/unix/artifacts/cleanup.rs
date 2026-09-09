use std::{
    fs::{self, Metadata},
    io::ErrorKind,
    os::unix::fs::FileTypeExt as _,
    path::Path,
};

use rustix::fs::{AtFlags, CWD, RenameFlags, linkat, renameat_with, unlinkat};

use super::{FileIdentity, private_directory::PrivateDirectory};
use crate::IpcError;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) enum ArtifactKind {
    Socket,
    SocketPublication,
    Marker,
}

impl ArtifactKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Socket => "socket",
            Self::SocketPublication => "socket publication",
            Self::Marker => "owner marker",
        }
    }

    fn matches(self, metadata: &Metadata) -> bool {
        match self {
            Self::Socket | Self::SocketPublication => metadata.file_type().is_socket(),
            Self::Marker => metadata.file_type().is_file(),
        }
    }
}

pub(super) fn verify_identity(
    path: &Path,
    expected: FileIdentity,
    kind: ArtifactKind,
) -> Result<(), IpcError> {
    let metadata = fs::symlink_metadata(path)?;
    let actual = FileIdentity::from_metadata(&metadata);
    if !kind.matches(&metadata) || actual != expected {
        return Err(identity_changed(kind, expected, actual));
    }
    Ok(())
}

pub(super) fn remove_matching(
    path: &Path,
    expected: FileIdentity,
    kind: ArtifactKind,
) -> Result<(), IpcError> {
    remove_matching_with_hook(RemovalTarget::new(path, expected, kind), |_| Ok(()))
}

#[derive(Clone, Copy)]
struct RemovalTarget<'path> {
    path: &'path Path,
    expected: FileIdentity,
    kind: ArtifactKind,
}

impl<'path> RemovalTarget<'path> {
    const fn new(path: &'path Path, expected: FileIdentity, kind: ArtifactKind) -> Self {
        Self {
            path,
            expected,
            kind,
        }
    }
}

fn remove_matching_with_hook<Hook>(target: RemovalTarget<'_>, hook: Hook) -> Result<(), IpcError>
where
    Hook: FnOnce(&Path) -> Result<(), IpcError>,
{
    let RemovalTarget {
        path,
        expected,
        kind,
    } = target;
    let parent = path.parent().ok_or_else(|| IpcError::Io {
        source: std::io::Error::new(ErrorKind::InvalidInput, "artifact path has no parent"),
    })?;
    let quarantine = PrivateDirectory::create(parent, ".tracepress-cleanup-")?;
    let entry = "artifact";
    let quarantine_path = quarantine.path().join(entry);
    renameat_with(
        CWD,
        path,
        quarantine.descriptor(),
        entry,
        RenameFlags::NOREPLACE,
    )
    .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    let metadata = fs::symlink_metadata(&quarantine_path)?;
    let actual = FileIdentity::from_metadata(&metadata);
    if kind.matches(&metadata) && actual == expected {
        hook(&quarantine_path)?;
        let current = fs::symlink_metadata(&quarantine_path)?;
        let current_identity = FileIdentity::from_metadata(&current);
        if kind.matches(&current) && current_identity == expected {
            unlinkat(quarantine.descriptor(), entry, AtFlags::empty())
                .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
            return Ok(());
        }
        restore_foreign(&quarantine, entry, path)?;
        return Err(identity_changed(kind, expected, current_identity));
    }
    restore_foreign(&quarantine, entry, path)?;
    Err(identity_changed(kind, expected, actual))
}

fn restore_foreign(
    quarantine: &PrivateDirectory,
    entry: &str,
    original: &Path,
) -> Result<(), IpcError> {
    match linkat(
        quarantine.descriptor(),
        entry,
        CWD,
        original,
        AtFlags::empty(),
    ) {
        Ok(()) => Ok(()),
        Err(error) if error == rustix::io::Errno::EXIST => Ok(()),
        Err(error) => Err(std::io::Error::from_raw_os_error(error.raw_os_error()).into()),
    }
}

pub(super) const fn identity_changed(
    kind: ArtifactKind,
    expected: FileIdentity,
    actual: FileIdentity,
) -> IpcError {
    IpcError::SocketArtifactChanged {
        artifact: kind.label(),
        expected_device: expected.device,
        expected_inode: expected.inode,
        actual_device: actual.device,
        actual_inode: actual.inode,
    }
}
