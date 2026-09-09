//! Bounded metadata strings for the durable and IPC allowlist.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::ContextDigest;

/// A bounded metadata string that records its own truncation.
///
/// The Phase 3 allowlist admits a few short structural strings — a `semantic_path`, a tool name,
/// an estimator identity — and nothing else. Every one of them is derived from request bytes or
/// analyzer identity, so it is bounded on the way in rather than trusted: an over-long value is
/// truncated at a character boundary, flagged, and accompanied by a digest of the complete value
/// so equal originals stay comparable without persisting them.
///
/// Its [`Debug`] representation reports the length, the truncation, and the digest, never the
/// characters: object keys chosen by an untrusted request reach a `semantic_path`, and the
/// privacy boundary keeps decoded request text out of logs.
#[derive(Clone, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct BoundedMetadataText {
    /// The retained value, never longer than the bound of its metadata field.
    value: String,
    /// The complete length of the original value in bytes.
    original_bytes: u64,
    /// A digest of the complete original value, present only when the value was truncated.
    full_value_hash: Option<ContextDigest>,
}

impl BoundedMetadataText {
    /// Largest retained `semantic_path`, in bytes.
    ///
    /// A JSON-Pointer-shaped path is a convenience for humans reading a block; the authoritative
    /// structural identity of a block is its raw span, so a long path is truncated rather than
    /// enlarging every block record on a wire whose body bound stays 32 KiB.
    pub const SEMANTIC_PATH_MAX_BYTES: usize = 256;
    /// Largest retained tool name, in bytes.
    pub const TOOL_NAME_MAX_BYTES: usize = 128;
    /// Largest retained estimator or encoding identity, in bytes.
    pub const ESTIMATOR_IDENTITY_MAX_BYTES: usize = 64;
    /// Largest value any metadata field may retain, in bytes.
    ///
    /// A transported value above this bound is rejected while deserializing, so a remote writer
    /// cannot enlarge a record past the bound its producer applied.
    pub const MAX_BYTES: usize = Self::SEMANTIC_PATH_MAX_BYTES;

    /// Bounds one `semantic_path` produced by the span indexer.
    #[must_use]
    pub fn semantic_path(value: &str) -> Self {
        Self::bounded(value, Self::SEMANTIC_PATH_MAX_BYTES)
    }

    /// Bounds one tool name observed in a tool definition, call, or result.
    #[must_use]
    pub fn tool_name(value: &str) -> Self {
        Self::bounded(value, Self::TOOL_NAME_MAX_BYTES)
    }

    /// Bounds one estimator or encoding identity.
    #[must_use]
    pub fn estimator_identity(value: &str) -> Self {
        Self::bounded(value, Self::ESTIMATOR_IDENTITY_MAX_BYTES)
    }

    /// Returns the retained, bounded value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns whether the original value did not fit its bound.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.full_value_hash.is_some()
    }

    /// Returns the complete length of the original value in bytes.
    #[must_use]
    pub const fn original_bytes(&self) -> u64 {
        self.original_bytes
    }

    /// Returns the digest of the complete original value, present only for a truncated value.
    #[must_use]
    pub const fn full_value_hash(&self) -> Option<ContextDigest> {
        self.full_value_hash
    }

    fn bounded(value: &str, max_bytes: usize) -> Self {
        let original_bytes = u64::try_from(value.len()).unwrap_or(u64::MAX);
        if value.len() <= max_bytes {
            return Self {
                value: value.to_owned(),
                original_bytes,
                full_value_hash: None,
            };
        }

        let mut retained = String::with_capacity(max_bytes);
        for character in value.chars() {
            if retained.len().saturating_add(character.len_utf8()) > max_bytes {
                break;
            }
            retained.push(character);
        }
        Self {
            value: retained,
            original_bytes,
            full_value_hash: Some(ContextDigest::from_bytes(value.as_bytes())),
        }
    }

    fn validated(fields: TransportedMetadataText) -> Result<Self, MetadataTextError> {
        let retained_bytes = u64::try_from(fields.value.len()).unwrap_or(u64::MAX);
        if fields.value.len() > Self::MAX_BYTES {
            return Err(MetadataTextError::AboveBound {
                retained_bytes,
                maximum_bytes: Self::MAX_BYTES,
            });
        }
        if fields.original_bytes < retained_bytes {
            return Err(MetadataTextError::OriginalShorterThanRetained {
                retained_bytes,
                original_bytes: fields.original_bytes,
            });
        }
        let truncated = fields.original_bytes > retained_bytes;
        if truncated != fields.full_value_hash.is_some() {
            return Err(MetadataTextError::TruncationDigestMismatch {
                truncated,
                digest_present: fields.full_value_hash.is_some(),
            });
        }
        Ok(Self {
            value: fields.value,
            original_bytes: fields.original_bytes,
            full_value_hash: fields.full_value_hash,
        })
    }
}

impl fmt::Debug for BoundedMetadataText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedMetadataText")
            .field("retained_bytes", &self.value.len())
            .field("original_bytes", &self.original_bytes)
            .field("truncated", &self.is_truncated())
            .field("full_value_hash", &self.full_value_hash)
            .finish()
    }
}

impl<'de> Deserialize<'de> for BoundedMetadataText {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        Self::validated(TransportedMetadataText::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

/// The transported representation, revalidated before it becomes a bounded value.
#[derive(Deserialize)]
struct TransportedMetadataText {
    value: String,
    original_bytes: u64,
    full_value_hash: Option<ContextDigest>,
}

/// Failure to accept a transported bounded metadata string.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataTextError {
    /// The retained value was longer than any metadata field may hold.
    #[error("metadata value retains {retained_bytes} bytes above the {maximum_bytes} byte bound")]
    AboveBound {
        /// The rejected retained length.
        retained_bytes: u64,
        /// The largest length any metadata field may retain.
        maximum_bytes: usize,
    },
    /// The declared original length was shorter than the retained value.
    #[error("metadata value retains {retained_bytes} bytes of a {original_bytes} byte original")]
    OriginalShorterThanRetained {
        /// The retained length.
        retained_bytes: u64,
        /// The rejected declared original length.
        original_bytes: u64,
    },
    /// Truncation and the digest of the complete value disagreed.
    #[error("metadata truncation {truncated} disagrees with digest presence {digest_present}")]
    TruncationDigestMismatch {
        /// Whether the declared lengths describe a truncated value.
        truncated: bool,
        /// Whether a digest of the complete value was present.
        digest_present: bool,
    },
}

/// A `semantic_path` above its bound must remain comparable through its digest, so the bound is
/// checked against the transport bound while compiling.
const _: () =
    assert!(BoundedMetadataText::SEMANTIC_PATH_MAX_BYTES <= BoundedMetadataText::MAX_BYTES);
/// A tool name shares the transport bound with every other metadata field.
const _: () = assert!(BoundedMetadataText::TOOL_NAME_MAX_BYTES <= BoundedMetadataText::MAX_BYTES);
/// An estimator identity shares the transport bound with every other metadata field.
const _: () =
    assert!(BoundedMetadataText::ESTIMATOR_IDENTITY_MAX_BYTES <= BoundedMetadataText::MAX_BYTES);
