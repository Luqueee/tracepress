use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;

use crate::ContentId;

/// Immutable bytes captured at a Tracepress content boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RawContent(Box<[u8]>);

impl RawContent {
    /// Copies exact bytes into immutable owned storage.
    #[must_use]
    pub fn new(raw_bytes: &[u8]) -> Self {
        Self(raw_bytes.into())
    }

    /// Borrows the exact stored bytes without decoding them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Transfers ownership of the exact stored bytes.
    #[must_use]
    pub fn into_boxed_bytes(self) -> Box<[u8]> {
        self.0
    }
}

impl AsRef<[u8]> for RawContent {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Logical classification of content without conflating text, binary, or unknown bytes.
#[allow(
    clippy::exhaustive_enums,
    reason = "the canonical schema requires consumers to handle every content kind"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    /// Human-readable plain text.
    Text,
    /// One JSON value.
    Json,
    /// Newline-delimited JSON values.
    Ndjson,
    /// Structured or unstructured log output.
    Log,
    /// Search or symbol lookup results.
    SearchResults,
    /// Test runner output.
    TestResults,
    /// Source code.
    SourceCode,
    /// A textual or binary diff.
    Diff,
    /// Image data.
    Image,
    /// Document data.
    Document,
    /// Known non-text bytes.
    Binary,
    /// Content whose kind is not known.
    Unknown,
}

/// Immutable content plus its identity and logical classification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContentObject {
    content_id: ContentId,
    raw_bytes: RawContent,
    content_kind: ContentKind,
}

impl ContentObject {
    /// Creates content identified from the exact supplied bytes.
    #[must_use]
    pub fn new(raw_bytes: &[u8], content_kind: ContentKind) -> Self {
        Self {
            content_id: ContentId::from_bytes(raw_bytes),
            raw_bytes: RawContent::new(raw_bytes),
            content_kind,
        }
    }

    /// Reconstructs persisted content while validating its declared identity.
    ///
    /// # Errors
    /// Returns [`ContentObjectError`] when `content_id` does not match `raw_bytes`.
    pub fn from_parts(
        content_id: ContentId,
        raw_bytes: RawContent,
        content_kind: ContentKind,
    ) -> Result<Self, ContentObjectError> {
        let computed = ContentId::from_bytes(raw_bytes.as_bytes());
        if content_id == computed {
            Ok(Self {
                content_id,
                raw_bytes,
                content_kind,
            })
        } else {
            Err(ContentObjectError::ContentIdMismatch {
                declared: content_id,
                computed,
            })
        }
    }

    /// Returns the SHA-256 identity of the exact stored bytes.
    #[must_use]
    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }

    /// Borrows the exact raw bytes.
    #[must_use]
    pub fn raw_bytes(&self) -> &[u8] {
        self.raw_bytes.as_bytes()
    }

    /// Returns the logical content classification.
    #[must_use]
    pub const fn kind(&self) -> ContentKind {
        self.content_kind
    }
}

impl<'de> Deserialize<'de> for ContentObject {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let serialized = SerializedContentObject::deserialize(deserializer)?;
        Self::from_parts(
            serialized.content_id,
            serialized.raw_bytes,
            serialized.content_kind,
        )
        .map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
struct SerializedContentObject {
    content_id: ContentId,
    raw_bytes: RawContent,
    content_kind: ContentKind,
}

/// Failure to reconstruct an immutable content object.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ContentObjectError {
    /// Persisted bytes did not match their declared SHA-256 identity.
    #[error("declared content ID {declared} does not match computed content ID {computed}")]
    ContentIdMismatch {
        /// The identity supplied by the serialized representation.
        declared: ContentId,
        /// The identity computed from the exact raw bytes.
        computed: ContentId,
    },
}
