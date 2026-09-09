#![allow(
    clippy::redundant_pub_crate,
    reason = "the crate-level tests and sibling blob-store modules share these internal seams"
)]
#![allow(
    clippy::too_many_arguments,
    reason = "crash-safe publication keeps writer, request, and optional failpoint explicit"
)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use tracepress_core::ContentId;

use super::codec::{self, BoundedWriter};
use super::database::{self, BlobRecord, ExternalRegistration};
use super::publication::{self, Publication};
use super::{
    BlobByteLimit, BlobError, BlobPut, BlobPutReceipt, BlobStorage, BlobStoreConfig,
    PersistenceCompression,
};
use crate::{Durability, StorageWriter};

pub(crate) const OBJECTS_DIRECTORY: &str = "objects";
const RAW_EXTENSION: &str = "blob";
const ZSTD_EXTENSION: &str = "zst";
const WRITE_CHUNK_BYTES: usize = 64 * 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Encoding {
    Raw,
    Zstd,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CrashPoint {
    TempWritten,
    FileSynced,
    Published,
    DirectorySynced,
    Registered,
}

impl Encoding {
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Raw => RAW_EXTENSION,
            Self::Zstd => ZSTD_EXTENSION,
        }
    }
}

/// Filesystem content-addressed backend rooted in one private directory.
#[derive(Clone, Debug)]
pub struct FilesystemCasBlobStore {
    pub(crate) root: PathBuf,
    pub(crate) max_bytes: BlobByteLimit,
    pub(crate) max_persisted_bytes: BlobByteLimit,
    compression: PersistenceCompression,
    durability: Durability,
}

impl FilesystemCasBlobStore {
    pub(crate) async fn open(
        config: BlobStoreConfig,
        durability: Durability,
    ) -> Result<Self, BlobError> {
        let root = config.root().to_path_buf();
        let objects = root.join(OBJECTS_DIRECTORY);
        let strict = durability == Durability::Strict;
        tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&objects)
                .map_err(|source| BlobError::io("create CAS directory", &objects, source))?;
            if strict {
                publication::sync_directory(&objects)?;
                publication::sync_directory(&root)?;
            }
            Ok::<(), BlobError>(())
        })
        .await
        .map_err(|source| BlobError::Database(crate::StorageError::ShutdownJoin(source)))??;
        Ok(Self {
            root: config.root().to_path_buf(),
            max_bytes: config.max_bytes(),
            max_persisted_bytes: config.max_persisted_bytes(),
            compression: config.compression(),
            durability,
        })
    }

    pub(crate) const fn max_bytes(&self) -> BlobByteLimit {
        self.max_bytes
    }

    pub(crate) async fn put(
        &self,
        writer: &StorageWriter,
        request: BlobPut<'_>,
    ) -> Result<BlobPutReceipt, BlobError> {
        self.put_inner(
            writer,
            request,
            #[cfg(test)]
            None,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn put_with_crash(
        &self,
        writer: &StorageWriter,
        request: BlobPut<'_>,
        boundary: CrashPoint,
    ) -> Result<BlobPutReceipt, BlobError> {
        self.put_inner(writer, request, Some(boundary)).await
    }

    async fn put_inner(
        &self,
        writer: &StorageWriter,
        request: BlobPut<'_>,
        #[cfg(test)] crash: Option<CrashPoint>,
    ) -> Result<BlobPutReceipt, BlobError> {
        let raw_bytes = codec::copy_bounded(request.raw_bytes, self.max_bytes)?;
        let backend = self.clone();
        let staged = tokio::task::spawn_blocking(move || {
            backend.publish(
                &raw_bytes,
                #[cfg(test)]
                crash,
            )
        })
        .await
        .map_err(|source| BlobError::Database(crate::StorageError::ShutdownJoin(source)))??;
        let record = writer
            .blob_register_external(ExternalRegistration {
                content_id: staged.content_id,
                external_ref: staged.external_ref,
                byte_length: staged.byte_length,
                kind: request.kind,
                created_at: request.created_at.to_owned(),
            })
            .await?;
        let receipt = match record {
            BlobRecord::Inline { raw_bytes } => {
                database::receipt_for_record(staged.content_id, BlobRecord::Inline { raw_bytes })
            }
            BlobRecord::External {
                external_ref,
                byte_length,
            } => {
                let _validated = self
                    .get_registered(staged.content_id, &external_ref, byte_length)
                    .await?;
                Ok(BlobPutReceipt::new(
                    staged.content_id,
                    BlobStorage::External,
                ))
            }
        }?;
        #[cfg(test)]
        crash_after(crash, CrashPoint::Registered, "database registration")?;
        Ok(receipt)
    }

    pub(crate) fn absolute_path(&self, content_id: ContentId, encoding: Encoding) -> PathBuf {
        self.root.join(Self::relative_path(content_id, encoding))
    }

    pub(crate) fn relative_path(content_id: ContentId, encoding: Encoding) -> PathBuf {
        let digest = content_id.to_string();
        let [first_byte, second_byte, ..] = content_id.as_bytes();
        let first = format!("{first_byte:02x}");
        let second = format!("{second_byte:02x}");
        PathBuf::from(OBJECTS_DIRECTORY)
            .join(first)
            .join(second)
            .join(format!("{digest}.{}", encoding.extension()))
    }

    fn publish(
        &self,
        raw_bytes: &[u8],
        #[cfg(test)] crash: Option<CrashPoint>,
    ) -> Result<StagedBlob, BlobError> {
        let encoding = match self.compression {
            PersistenceCompression::Disabled => Encoding::Raw,
            PersistenceCompression::Zstd(_) => Encoding::Zstd,
        };
        let mut hasher = Sha256::new();
        let provisional_parent = self.root.join(OBJECTS_DIRECTORY);
        let mut temporary = NamedTempFile::new_in(&provisional_parent)
            .map_err(|source| BlobError::io("create CAS temporary", &provisional_parent, source))?;
        match self.compression {
            PersistenceCompression::Disabled => {
                write_chunks(&mut temporary, raw_bytes, &mut hasher)?;
            }
            PersistenceCompression::Zstd(level) => {
                let bounded = BoundedWriter::new(temporary, self.max_persisted_bytes);
                let mut encoder =
                    zstd::stream::write::Encoder::new(bounded, level.get()).map_err(|source| {
                        BlobError::io("initialize zstd encoder", &provisional_parent, source)
                    })?;
                encoder
                    .set_pledged_src_size(Some(raw_length(raw_bytes)?))
                    .map_err(|source| {
                        BlobError::io("set zstd content size", &provisional_parent, source)
                    })?;
                encoder.include_checksum(true).map_err(|source| {
                    BlobError::io("enable zstd checksum", &provisional_parent, source)
                })?;
                encoder
                    .window_log(codec::zstd_window_log_max())
                    .map_err(|source| {
                        BlobError::io("bound zstd encoder", &provisional_parent, source)
                    })?;
                write_chunks(&mut encoder, raw_bytes, &mut hasher)?;
                temporary = encoder
                    .finish()
                    .map_err(|source| {
                        BlobError::io("finish zstd frame", &provisional_parent, source)
                    })?
                    .into_inner();
            }
        }
        temporary
            .flush()
            .map_err(|source| BlobError::io("flush CAS temporary", temporary.path(), source))?;
        #[cfg(test)]
        crash_after(crash, CrashPoint::TempWritten, "temporary write")?;
        if self.durability == Durability::Strict {
            temporary
                .as_file()
                .sync_all()
                .map_err(|source| BlobError::io("fsync CAS temporary", temporary.path(), source))?;
        }
        #[cfg(test)]
        crash_after(crash, CrashPoint::FileSynced, "file fsync")?;
        let content_id = codec::content_id(hasher)?;
        let relative_ref = Self::relative_path(content_id, encoding);
        let final_path = self.root.join(&relative_ref);
        let parent = final_path.parent().ok_or(BlobError::Corrupt {
            content_id,
            detail: "CAS path has no parent",
        })?;
        fs::create_dir_all(parent)
            .map_err(|source| BlobError::io("create digest shard", parent, source))?;
        let publication = publication::publish_noreplace(temporary, &final_path)?;
        #[cfg(test)]
        crash_after(crash, CrashPoint::Published, "atomic publication")?;
        if publication == Publication::Existing {
            let _existing =
                self.read_path_blocking(&final_path, encoding, Some(raw_length(raw_bytes)?))?;
        } else if self.durability == Durability::Strict {
            publication::sync_directory(parent)?;
        }
        #[cfg(test)]
        crash_after(crash, CrashPoint::DirectorySynced, "parent directory fsync")?;
        Ok(StagedBlob {
            content_id,
            external_ref: path_string(&relative_ref, content_id)?,
            byte_length: raw_length(raw_bytes)?,
        })
    }
}

#[cfg(test)]
fn crash_after(
    configured: Option<CrashPoint>,
    current: CrashPoint,
    boundary: &'static str,
) -> Result<(), BlobError> {
    if configured == Some(current) {
        Err(BlobError::InjectedCrash { boundary })
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct StagedBlob {
    content_id: ContentId,
    external_ref: String,
    byte_length: u64,
}

fn write_chunks(
    writer: &mut impl std::io::Write,
    raw_bytes: &[u8],
    hasher: &mut Sha256,
) -> Result<(), BlobError> {
    for chunk in raw_bytes.chunks(WRITE_CHUNK_BYTES) {
        hasher.update(chunk);
        writer.write_all(chunk).map_err(|source| {
            BlobError::io("write CAS temporary", Path::new("<temporary>"), source)
        })?;
    }
    Ok(())
}

fn path_string(path: &Path, content_id: ContentId) -> Result<String, BlobError> {
    path.to_str().map(str::to_owned).ok_or(BlobError::Corrupt {
        content_id,
        detail: "CAS root produced a non-UTF-8 relative reference",
    })
}

fn raw_length(raw_bytes: &[u8]) -> Result<u64, BlobError> {
    u64::try_from(raw_bytes.len()).map_err(|_error| BlobError::LimitUnrepresentable {
        field: "raw_blob_length",
        value: u64::MAX,
    })
}
