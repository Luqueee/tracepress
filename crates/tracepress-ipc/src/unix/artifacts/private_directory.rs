use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

use crate::IpcError;

#[derive(Debug)]
pub(super) struct PrivateDirectory {
    descriptor: File,
    path: PathBuf,
}

impl PrivateDirectory {
    pub(super) fn create(parent: &Path, prefix: &str) -> Result<Self, IpcError> {
        let path = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(parent)?
            .keep();
        let descriptor = File::open(&path)?;
        Ok(Self { descriptor, path })
    }

    pub(super) const fn descriptor(&self) -> &File {
        &self.descriptor
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        if fs::remove_dir(&self.path).is_err() {}
    }
}
