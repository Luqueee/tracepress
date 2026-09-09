#![allow(
    clippy::redundant_pub_crate,
    reason = "filesystem publication uses these helpers across the private module boundary"
)]
#![allow(
    clippy::needless_pass_by_value,
    reason = "publication consumes the temporary file on non-Unix platforms"
)]

use std::fs::File;
use std::path::Path;

use tempfile::NamedTempFile;

use super::BlobError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Publication {
    Published,
    Existing,
}

pub(crate) fn publish_noreplace(
    temporary: NamedTempFile,
    final_path: &Path,
) -> Result<Publication, BlobError> {
    publish_platform(temporary, final_path)
}

#[cfg(unix)]
fn publish_platform(temporary: NamedTempFile, final_path: &Path) -> Result<Publication, BlobError> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    match renameat_with(
        CWD,
        temporary.path(),
        CWD,
        final_path,
        RenameFlags::NOREPLACE,
    ) {
        Ok(()) => Ok(Publication::Published),
        Err(error) if error == rustix::io::Errno::EXIST => Ok(Publication::Existing),
        Err(error) => Err(BlobError::io(
            "atomically publish",
            final_path,
            error.into(),
        )),
    }
}

#[cfg(not(unix))]
fn publish_platform(temporary: NamedTempFile, final_path: &Path) -> Result<Publication, BlobError> {
    match temporary.persist_noclobber(final_path) {
        Ok(_file) => Ok(Publication::Published),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(Publication::Existing)
        }
        Err(error) => Err(BlobError::io("atomically publish", final_path, error.error)),
    }
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), BlobError> {
    let directory = File::open(path)
        .map_err(|source| BlobError::io("open directory for fsync", path, source))?;
    directory
        .sync_all()
        .map_err(|source| BlobError::io("fsync directory", path, source))
}
