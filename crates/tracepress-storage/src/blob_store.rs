mod codec;
mod config;
pub(crate) mod database;
mod error;
pub(crate) mod filesystem;
mod gc;
mod publication;
mod retrieval;

use std::future::Future;

use tokio::sync::Mutex;
use tracepress_core::{ContentId, ContentKind, RawContent};

pub use config::{
    BlobByteLimit, BlobStoreConfig, InlineBlobMaxBytes, PersistenceCompression, ZstdLevel,
};
pub use error::BlobError;
pub use filesystem::FilesystemCasBlobStore;
pub use gc::{DaemonGc, GcDecision, GcReport};

use crate::{StorageConfig, StorageWriter};

/// Exact bytes and metadata submitted to a blob store.
#[derive(Clone, Copy, Debug)]
pub struct BlobPut<'a> {
    raw_bytes: &'a [u8],
    kind: ContentKind,
    created_at: &'a str,
}

impl<'a> BlobPut<'a> {
    /// Creates a borrowed exact-byte put request.
    #[must_use]
    pub const fn new(raw_bytes: &'a [u8], kind: ContentKind, created_at: &'a str) -> Self {
        Self {
            raw_bytes,
            kind,
            created_at,
        }
    }
}

/// Physical location selected for a content object.
#[allow(
    clippy::exhaustive_enums,
    reason = "the hybrid store has exactly the two schema-backed locations"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlobStorage {
    /// Exact bytes are in the `content_objects.raw_bytes` column.
    Inline,
    /// Exact or persistence-compressed bytes are in the filesystem CAS.
    External,
}

/// Durable identity and location returned after a put commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobPutReceipt {
    content_id: ContentId,
    storage: BlobStorage,
}

impl BlobPutReceipt {
    pub(crate) const fn new(content_id: ContentId, storage: BlobStorage) -> Self {
        Self {
            content_id,
            storage,
        }
    }

    /// Returns the SHA-256 identity of the original uncompressed bytes.
    #[must_use]
    pub const fn content_id(self) -> ContentId {
        self.content_id
    }

    /// Returns the selected physical backend.
    #[must_use]
    pub const fn storage(self) -> BlobStorage {
        self.storage
    }
}

/// Exact content persistence and pinning boundary.
pub trait BlobStore {
    /// Persists exact bytes without taking ownership from the caller.
    fn put(
        &self,
        request: BlobPut<'_>,
    ) -> impl Future<Output = Result<BlobPutReceipt, BlobError>> + Send;

    /// Retrieves and validates exact original bytes.
    fn get(
        &self,
        content_id: ContentId,
    ) -> impl Future<Output = Result<RawContent, BlobError>> + Send;

    /// Adds one checked durable pin.
    fn pin(&self, content_id: ContentId) -> impl Future<Output = Result<u64, BlobError>> + Send;

    /// Removes one checked durable pin without allowing a negative count.
    fn unpin(&self, content_id: ContentId) -> impl Future<Output = Result<u64, BlobError>> + Send;
}

/// Inline backend marker; all operations execute on the sole writer connection.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct InlineSqliteBlobStore;

/// Hybrid exact-byte store composed from the inline writer and filesystem CAS.
#[derive(Debug)]
pub struct HybridBlobStore {
    writer: StorageWriter,
    filesystem: FilesystemCasBlobStore,
    inline_max: InlineBlobMaxBytes,
    cas_guard: Mutex<()>,
}

impl HybridBlobStore {
    /// Opens the CAS and the daemon-owned `SQLite` writer.
    ///
    /// # Errors
    /// Returns a typed storage or filesystem error when either backend cannot be opened.
    pub async fn open(storage: StorageConfig, blobs: BlobStoreConfig) -> Result<Self, BlobError> {
        let durability = storage.durability();
        let inline_max = blobs.inline_max();
        let filesystem = FilesystemCasBlobStore::open(blobs, durability).await?;
        let writer = StorageWriter::open(storage).await?;
        Ok(Self {
            writer,
            filesystem,
            inline_max,
            cas_guard: Mutex::new(()),
        })
    }

    /// Borrows the capability required to run orphan collection.
    #[must_use]
    pub const fn daemon_gc(&self) -> DaemonGc<'_> {
        DaemonGc::new(self)
    }

    /// Drains and joins the sole writer thread.
    ///
    /// # Errors
    /// Returns a typed shutdown error if the writer cannot drain or join cleanly.
    pub async fn shutdown(self) -> Result<(), BlobError> {
        self.writer.shutdown().await.map_err(Into::into)
    }

    pub(crate) const fn writer(&self) -> &StorageWriter {
        &self.writer
    }

    pub(crate) const fn filesystem(&self) -> &FilesystemCasBlobStore {
        &self.filesystem
    }

    pub(crate) const fn cas_guard(&self) -> &Mutex<()> {
        &self.cas_guard
    }

    #[cfg(test)]
    pub(crate) async fn put_with_crash(
        &self,
        request: BlobPut<'_>,
        boundary: filesystem::CrashPoint,
    ) -> Result<BlobPutReceipt, BlobError> {
        let _guard = self.cas_guard.lock().await;
        self.filesystem
            .put_with_crash(&self.writer, request, boundary)
            .await
    }
}

impl BlobStore for HybridBlobStore {
    async fn put(&self, request: BlobPut<'_>) -> Result<BlobPutReceipt, BlobError> {
        if request.raw_bytes.len() <= self.inline_max.get() {
            return InlineSqliteBlobStore::put(&self.writer, request, self.filesystem.max_bytes())
                .await;
        }
        let _guard = self.cas_guard.lock().await;
        self.filesystem.put(&self.writer, request).await
    }

    async fn get(&self, content_id: ContentId) -> Result<RawContent, BlobError> {
        let record = self.writer.blob_lookup(content_id).await?;
        match record {
            Some(database::BlobRecord::Inline { raw_bytes, .. }) => {
                codec::validate_raw(content_id, &raw_bytes)
            }
            Some(database::BlobRecord::External {
                external_ref,
                byte_length,
                ..
            }) => {
                let _guard = self.cas_guard.lock().await;
                self.filesystem
                    .get_registered(content_id, &external_ref, byte_length)
                    .await
            }
            None => {
                let _guard = self.cas_guard.lock().await;
                self.filesystem.get_orphan(content_id).await
            }
        }
    }

    async fn pin(&self, content_id: ContentId) -> Result<u64, BlobError> {
        self.writer
            .blob_pin(content_id, database::PinChange::Increment)
            .await
    }

    async fn unpin(&self, content_id: ContentId) -> Result<u64, BlobError> {
        self.writer
            .blob_pin(content_id, database::PinChange::Decrement)
            .await
    }
}

impl InlineSqliteBlobStore {
    async fn put(
        writer: &StorageWriter,
        request: BlobPut<'_>,
        max_bytes: BlobByteLimit,
    ) -> Result<BlobPutReceipt, BlobError> {
        let content_id = ContentId::from_bytes(request.raw_bytes);
        let raw_bytes = codec::copy_bounded(request.raw_bytes, max_bytes)?;
        let record = writer
            .blob_register_inline(database::InlineRegistration {
                content_id,
                raw_bytes,
                kind: request.kind,
                created_at: request.created_at.to_owned(),
            })
            .await?;
        database::receipt_for_record(content_id, record)
    }
}
