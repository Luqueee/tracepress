#![allow(
    clippy::redundant_pub_crate,
    reason = "filesystem and GC modules share these internal recovery helpers"
)]
#![allow(
    clippy::too_many_arguments,
    reason = "validated recovery keeps path, encoding, identity, and declared length explicit"
)]

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use tracepress_core::{ContentId, RawContent};

use super::BlobError;
use super::codec;
use super::filesystem::{Encoding, FilesystemCasBlobStore};

impl FilesystemCasBlobStore {
    pub(crate) async fn get_registered(
        &self,
        content_id: ContentId,
        external_ref: &str,
        byte_length: u64,
    ) -> Result<RawContent, BlobError> {
        let (path, encoding) = self.validate_reference(content_id, external_ref)?;
        self.read_path(path, encoding, Some(byte_length)).await
    }

    pub(crate) async fn get_orphan(&self, content_id: ContentId) -> Result<RawContent, BlobError> {
        for encoding in [Encoding::Raw, Encoding::Zstd] {
            let path = self.absolute_path(content_id, encoding);
            match fs::metadata(&path) {
                Ok(_metadata) => return self.read_path(path, encoding, None).await,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(BlobError::io("inspect orphan", &path, source)),
            }
        }
        Err(BlobError::Missing { content_id })
    }

    async fn read_path(
        &self,
        path: PathBuf,
        encoding: Encoding,
        expected: Option<u64>,
    ) -> Result<RawContent, BlobError> {
        let backend = self.clone();
        tokio::task::spawn_blocking(move || backend.read_path_blocking(&path, encoding, expected))
            .await
            .map_err(|source| BlobError::Database(crate::StorageError::ShutdownJoin(source)))?
    }

    pub(crate) fn read_path_blocking(
        &self,
        path: &Path,
        encoding: Encoding,
        expected: Option<u64>,
    ) -> Result<RawContent, BlobError> {
        let content_id = content_id_from_path(path)?;
        let file = File::open(path).map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => BlobError::Missing { content_id },
            _ => BlobError::io("open CAS object", path, source),
        })?;
        let persisted_length = file
            .metadata()
            .map_err(|source| BlobError::io("inspect CAS object", path, source))?
            .len();
        if persisted_length > self.max_persisted_bytes.get_u64() {
            return Err(BlobError::TooLarge {
                content_id,
                actual: persisted_length,
                maximum: self.max_persisted_bytes.get_u64(),
            });
        }
        match encoding {
            Encoding::Raw => codec::read_raw(file, content_id, expected, self.max_bytes),
            Encoding::Zstd => codec::read_zstd(file, content_id, expected, self.max_bytes),
        }
    }

    fn validate_reference(
        &self,
        content_id: ContentId,
        external_ref: &str,
    ) -> Result<(PathBuf, Encoding), BlobError> {
        for encoding in [Encoding::Raw, Encoding::Zstd] {
            let expected = Self::relative_path(content_id, encoding);
            if expected == Path::new(external_ref) {
                return Ok((self.root.join(expected), encoding));
            }
        }
        Err(BlobError::Corrupt {
            content_id,
            detail: "database external reference is not the canonical digest path",
        })
    }
}

pub(crate) fn content_id_from_path(path: &Path) -> Result<ContentId, BlobError> {
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or(BlobError::DigestEncoding)?;
    stem.parse().map_err(|_error| BlobError::DigestEncoding)
}
