use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use rustix::fs::{CWD, RenameFlags, renameat_with};

use super::{
    FileIdentity,
    cleanup::{ArtifactKind, identity_changed, verify_identity},
    private_directory::PrivateDirectory,
};
use crate::IpcError;

#[derive(Debug)]
pub(in crate::unix) struct StagedSocket {
    directory: PrivateDirectory,
    path: PathBuf,
}

impl StagedSocket {
    pub(in crate::unix) fn create(public: &Path) -> Result<Self, IpcError> {
        let parent = public.parent().ok_or_else(|| IpcError::Io {
            source: std::io::Error::new(ErrorKind::InvalidInput, "socket path has no parent"),
        })?;
        let directory = PrivateDirectory::create(parent, ".tracepress-bind-")?;
        let path = directory.path().join("socket");
        Ok(Self { directory, path })
    }

    pub(in crate::unix) fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::unix) fn publish(
        self,
        public: &Path,
        expected: FileIdentity,
    ) -> Result<(), IpcError> {
        match renameat_with(
            self.directory.descriptor(),
            "socket",
            CWD,
            public,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => verify_identity(public, expected, ArtifactKind::SocketPublication),
            Err(error) if error == rustix::io::Errno::EXIST => {
                let actual = FileIdentity::from_metadata(&fs::symlink_metadata(public)?);
                Err(identity_changed(
                    ArtifactKind::SocketPublication,
                    expected,
                    actual,
                ))
            }
            Err(error) => Err(std::io::Error::from_raw_os_error(error.raw_os_error()).into()),
        }
    }
}
