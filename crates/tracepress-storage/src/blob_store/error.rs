use std::path::PathBuf;

use thiserror::Error;
use tracepress_core::ContentId;

use crate::StorageError;

/// A typed exact-byte blob persistence or recovery failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BlobError {
    /// `SQLite` writer or transaction failure.
    #[error("blob database operation failed: {0}")]
    Database(#[from] StorageError),
    /// Filesystem operation failed at a known path.
    #[error("blob filesystem {operation} failed at {path}: {source}")]
    Filesystem {
        /// Stable operation name.
        operation: &'static str,
        /// Affected path.
        path: PathBuf,
        /// Operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// A configured limit was zero.
    #[error("blob limit {field} must be greater than zero")]
    ZeroLimit {
        /// Configuration field.
        field: &'static str,
    },
    /// A configured limit cannot be represented by this platform.
    #[error("blob limit {field} value {value} is not representable on this platform")]
    LimitUnrepresentable {
        /// Configuration field.
        field: &'static str,
        /// Rejected value.
        value: u64,
    },
    /// A blob exceeds its configured bound.
    #[error("blob {content_id} length {actual} exceeds configured maximum {maximum}")]
    TooLarge {
        /// Content identity, if hashing completed.
        content_id: ContentId,
        /// Observed or declared byte count.
        actual: u64,
        /// Configured bound.
        maximum: u64,
    },
    /// A bounded allocation could not be reserved.
    #[error("unable to reserve {requested} bytes for blob processing")]
    Allocation {
        /// Requested allocation size.
        requested: usize,
    },
    /// The requested identity has neither a database row nor recoverable CAS object.
    #[error("blob {content_id} is missing")]
    Missing {
        /// Requested content identity.
        content_id: ContentId,
    },
    /// Persisted bytes or metadata do not match the declared identity.
    #[error("blob {content_id} is corrupt: {detail}")]
    Corrupt {
        /// Declared content identity.
        content_id: ContentId,
        /// Stable corruption description.
        detail: &'static str,
    },
    /// Pin removal would make the durable count negative.
    #[error("blob {content_id} has no pin to remove")]
    PinUnderflow {
        /// Requested content identity.
        content_id: ContentId,
    },
    /// Pin increment exceeds `SQLite`'s checked integer range.
    #[error("blob {content_id} pin count cannot be incremented")]
    PinOverflow {
        /// Requested content identity.
        content_id: ContentId,
    },
    /// Compression level is outside zstd's documented range.
    #[error("zstd persistence level {value} is outside 1..=22")]
    InvalidZstdLevel {
        /// Rejected level.
        value: i32,
    },
    /// SHA-256 output could not be represented as a core content identity.
    #[error("SHA-256 output could not be converted into a content identity")]
    DigestEncoding,
    /// A deterministic test failpoint interrupted the crash-safe sequence.
    #[cfg(test)]
    #[error("injected blob crash after {boundary}")]
    InjectedCrash {
        /// Last completed ordering boundary.
        boundary: &'static str,
    },
}

impl BlobError {
    pub(crate) fn io(
        operation: &'static str,
        path: &std::path::Path,
        source: std::io::Error,
    ) -> Self {
        Self::Filesystem {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}
