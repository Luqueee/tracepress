//! Durable `SQLite` storage boundary for Tracepress.

mod blob_store;
mod encode;
mod error;
mod models;
mod records;
mod schema;
mod writer;

#[cfg(test)]
mod blob_store_tests;

pub use blob_store::{
    BlobByteLimit, BlobError, BlobPut, BlobPutReceipt, BlobStorage, BlobStore, BlobStoreConfig,
    DaemonGc, FilesystemCasBlobStore, GcDecision, GcReport, HybridBlobStore, InlineBlobMaxBytes,
    InlineSqliteBlobStore, PersistenceCompression, ZstdLevel,
};
pub use error::StorageError;
pub use models::{
    FidelityClass, ObservationStatus, ProviderKind, ProviderProtocol, ProviderResponseState,
    WriteBatch, WriteCommand, WriteReceipt,
};
pub use schema::{ConnectionSettings, Durability};
pub use writer::{StorageConfig, StorageWriter};

#[cfg(test)]
mod capacity_tests;
#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod overflow_tests;
#[cfg(test)]
mod record_tests;
#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod writer_tests;
