use std::path::{Path, PathBuf};

use crate::blob_store::BlobError;

/// Maximum byte length routed into `SQLite`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InlineBlobMaxBytes(usize);

impl InlineBlobMaxBytes {
    /// Validates a platform-representable inline threshold.
    ///
    /// # Errors
    /// Returns an error when the threshold cannot be represented by this platform.
    pub fn new(value: u64) -> Result<Self, BlobError> {
        usize::try_from(value)
            .map(Self)
            .map_err(|_error| BlobError::LimitUnrepresentable {
                field: "inline_blob_max_bytes",
                value,
            })
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// Positive maximum accepted original or persisted blob length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobByteLimit {
    value: u64,
    platform_value: usize,
}

impl BlobByteLimit {
    /// Validates a positive platform-representable byte limit.
    ///
    /// # Errors
    /// Returns an error when the limit is zero or cannot be represented by this platform.
    pub fn new(value: u64) -> Result<Self, BlobError> {
        let platform_value =
            usize::try_from(value).map_err(|_error| BlobError::LimitUnrepresentable {
                field: "blob_max_bytes",
                value,
            })?;
        if platform_value == 0 {
            return Err(BlobError::ZeroLimit {
                field: "blob_max_bytes",
            });
        }
        Ok(Self {
            value,
            platform_value,
        })
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.platform_value
    }

    pub(crate) const fn get_u64(self) -> u64 {
        self.value
    }
}

/// Valid zstd persistence compression level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZstdLevel(i32);

impl ZstdLevel {
    /// Accepts zstd's documented levels 1 through 22.
    ///
    /// # Errors
    /// Returns an error when `value` is outside zstd's documented range.
    pub const fn new(value: i32) -> Result<Self, BlobError> {
        if value >= 1 && value <= 22 {
            Ok(Self(value))
        } else {
            Err(BlobError::InvalidZstdLevel { value })
        }
    }

    pub(crate) const fn get(self) -> i32 {
        self.0
    }
}

/// Optional persistence-only encoding for external CAS objects.
#[allow(
    clippy::exhaustive_enums,
    reason = "persistence is either exact raw bytes or one bounded zstd frame"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceCompression {
    /// Store original bytes directly.
    Disabled,
    /// Store one checksummed zstd frame while retaining raw-byte identity.
    Zstd(ZstdLevel),
}

/// Validated hybrid blob storage configuration.
#[derive(Clone, Debug)]
pub struct BlobStoreConfig {
    root: PathBuf,
    inline_max: InlineBlobMaxBytes,
    max_bytes: BlobByteLimit,
    max_persisted_bytes: BlobByteLimit,
    compression: PersistenceCompression,
}

impl BlobStoreConfig {
    /// Creates uncompressed CAS configuration with equal raw and persisted bounds.
    #[must_use]
    pub const fn new(
        root: PathBuf,
        inline_max: InlineBlobMaxBytes,
        max_bytes: BlobByteLimit,
    ) -> Self {
        Self {
            root,
            inline_max,
            max_bytes,
            max_persisted_bytes: max_bytes,
            compression: PersistenceCompression::Disabled,
        }
    }

    /// Enables bounded persistence-only compression.
    #[must_use]
    pub const fn with_compression(mut self, compression: PersistenceCompression) -> Self {
        self.compression = compression;
        self
    }

    /// Sets the independent maximum compressed object size.
    #[must_use]
    pub const fn with_max_persisted_bytes(mut self, limit: BlobByteLimit) -> Self {
        self.max_persisted_bytes = limit;
        self
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) const fn inline_max(&self) -> InlineBlobMaxBytes {
        self.inline_max
    }

    pub(crate) const fn max_bytes(&self) -> BlobByteLimit {
        self.max_bytes
    }

    pub(crate) const fn max_persisted_bytes(&self) -> BlobByteLimit {
        self.max_persisted_bytes
    }

    pub(crate) const fn compression(&self) -> PersistenceCompression {
        self.compression
    }
}
