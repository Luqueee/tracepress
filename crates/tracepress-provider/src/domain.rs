#![allow(
    clippy::redundant_pub_crate,
    reason = "sibling parser modules share this fail-open status mapping"
)]

//! Canonical provider-observation domain values.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Provider identity known to this parser.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderKind {
    /// `OpenAI`.
    OpenAi,
}

/// Versioned provider protocol identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderProtocol {
    /// `OpenAI` Responses API version 1.
    OpenAiResponsesV1,
}

/// Outcome of semantic observation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ObservationStatus {
    /// The complete bounded input was understood.
    Complete,
    /// Some known fields or events could not be understood.
    Partial,
    /// The input is not supported by this observer.
    Unsupported,
    /// The input is not valid for the protocol.
    Malformed,
    /// A configured semantic resource bound was reached.
    ResourceLimit,
    /// Observation work was dropped because its sink was full or closed.
    ObserverBackpressure,
    /// Observation was cancelled before completion.
    Cancelled,
}

/// Canonical lifecycle of a provider response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderResponseState {
    /// Provider accepted the request but has not started processing.
    Queued,
    /// Provider is producing a response.
    InProgress,
    /// Provider completed successfully.
    Completed,
    /// Provider stopped without completing all requested work.
    Incomplete,
    /// Provider reported an error.
    Failed,
    /// The client or observer cancelled processing.
    Cancelled,
    /// The upstream connection ended before a semantic terminal event.
    Disconnected,
    /// No trustworthy lifecycle state was present.
    Unknown,
}

/// Availability of provider token usage.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UsageStatus {
    /// A terminal response supplied a valid usage object.
    Final,
    /// Usage was supplied, but one or more values were absent or anomalous.
    Partial,
    /// No usable usage object was observed.
    Unavailable,
}

/// Policy controlling optional content capture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContentCaptureMode {
    /// Do not retain provider content.
    Off,
    /// Retain only the allowlisted semantic metadata.
    #[default]
    MetadataOnly,
    /// Explicit local-only raw capture capability.
    LocalRaw,
}

/// Version of the `OpenAI` Responses parser.
pub const OPENAI_RESPONSES_PARSER_VERSION: u32 = 1;
/// Version of the usage normalizer.
pub const USAGE_NORMALIZER_VERSION: u32 = 1;

/// A bounded exact copy of a provider `usage` object.
///
/// The bytes contain only the isolated JSON object, including the provider's original
/// whitespace and ordering. Its debug representation intentionally omits the bytes, while
/// serialization carries them verbatim so a remote writer stores the provider's own object.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct RawProviderUsage {
    bytes: Box<[u8]>,
}

impl RawProviderUsage {
    /// Constructs an exact raw usage value after checking its byte bound and JSON shape.
    ///
    /// # Errors
    ///
    /// Returns [`UsageError::ResourceLimit`] when `bytes` exceeds `max_bytes`, or
    /// [`UsageError::Malformed`] when it is not a JSON object.
    pub fn new(bytes: &[u8], max_bytes: usize) -> Result<Self, UsageError> {
        if bytes.len() > max_bytes {
            return Err(UsageError::ResourceLimit);
        }
        if bytes.first() != Some(&b'{')
            || serde_json::from_slice::<serde_json::Value>(bytes).is_err()
        {
            return Err(UsageError::Malformed);
        }
        Ok(Self {
            bytes: bytes.into(),
        })
    }

    /// Returns the exact usage-object bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the bounded byte length without exposing usage values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether this usage object has no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for RawProviderUsage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawProviderUsage")
            .field("len", &self.bytes.len())
            .finish()
    }
}

impl AsRef<[u8]> for RawProviderUsage {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Serialize for RawProviderUsage {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: serde::Serializer,
    {
        serializer.serialize_bytes(&self.bytes)
    }
}

impl<'de> Deserialize<'de> for RawProviderUsage {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        deserializer.deserialize_bytes(RawProviderUsageVisitor)
    }
}

/// Accepts the transported usage bytes from byte, text, and sequence encodings.
struct RawProviderUsageVisitor;

impl RawProviderUsageVisitor {
    /// Maximum sequence capacity trusted from an untrusted length hint.
    const HINT_CAPACITY: usize = 4096;

    fn accept<ErrorType>(bytes: &[u8]) -> Result<RawProviderUsage, ErrorType>
    where
        ErrorType: serde::de::Error,
    {
        // The sender's byte bound already applied; only the JSON-object invariant is rechecked.
        RawProviderUsage::new(bytes, bytes.len()).map_err(serde::de::Error::custom)
    }
}

impl<'de> serde::de::Visitor<'de> for RawProviderUsageVisitor {
    type Value = RawProviderUsage;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the exact bytes of one provider usage JSON object")
    }

    fn visit_bytes<ErrorType>(self, value: &[u8]) -> Result<Self::Value, ErrorType>
    where
        ErrorType: serde::de::Error,
    {
        Self::accept(value)
    }

    fn visit_str<ErrorType>(self, value: &str) -> Result<Self::Value, ErrorType>
    where
        ErrorType: serde::de::Error,
    {
        Self::accept(value.as_bytes())
    }

    fn visit_seq<SeqType>(self, mut sequence: SeqType) -> Result<Self::Value, SeqType::Error>
    where
        SeqType: serde::de::SeqAccess<'de>,
    {
        let mut bytes = Vec::with_capacity(
            sequence
                .size_hint()
                .unwrap_or_default()
                .min(Self::HINT_CAPACITY),
        );
        while let Some(byte) = sequence.next_element::<u8>()? {
            bytes.push(byte);
        }
        Self::accept(&bytes)
    }
}

/// Checked canonical usage values independent of a provider wire format.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct NormalizedUsage {
    /// Total input tokens reported by the provider.
    pub input_total: Option<u64>,
    /// Cached input tokens reported by the provider.
    pub input_cached: Option<u64>,
    /// Input tokens not served from cache, when derivable safely.
    pub input_uncached: Option<u64>,
    /// Cache-write tokens, when reported.
    pub cache_write: Option<u64>,
    /// Total output tokens reported by the provider.
    pub output_total: Option<u64>,
    /// Reasoning output tokens reported by the provider.
    pub output_reasoning: Option<u64>,
    /// Provider-reported total tokens.
    pub total: Option<u64>,
    /// Completeness of this normalized result.
    pub status: UsageStatus,
    /// Version of the normalization rules.
    pub normalizer_version: u32,
    /// Explicit consistency or input anomalies.
    pub anomalies: AnomalyFlags,
}

/// Explicit usage-normalization anomalies. Unknown values remain absent.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Each independently observable provider usage anomaly must remain queryable."
)]
pub struct AnomalyFlags {
    /// A numeric field had the wrong JSON type or a negative value.
    pub invalid_number: bool,
    /// A numeric value could not fit in an unsigned 64-bit integer.
    pub overflow: bool,
    /// Cached input exceeded total input.
    pub cached_exceeds_input: bool,
    /// Reasoning output exceeded total output.
    pub reasoning_exceeds_output: bool,
    /// Reported total was inconsistent with available components.
    pub inconsistent_total: bool,
}

impl AnomalyFlags {
    /// Returns whether any anomaly was recorded.
    #[must_use]
    pub const fn any(self) -> bool {
        self.invalid_number
            || self.overflow
            || self.cached_exceeds_input
            || self.reasoning_exceeds_output
            || self.inconsistent_total
    }
}

/// Failure while retaining or interpreting a usage object.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum UsageError {
    /// Usage bytes exceeded the configured bound.
    #[error("usage object exceeded the configured resource limit")]
    ResourceLimit,
    /// Usage bytes were not a JSON object.
    #[error("usage object is malformed JSON")]
    Malformed,
}

/// Largest provider usage object any bound may retain, in bytes.
///
/// `provider_usage.raw_usage_json` is declared `CHECK(length(raw_usage_json) <= 65536)`, so
/// retaining a larger object would only parse usage the durable schema must then reject, rolling
/// back the whole provider batch. Every caller that persists an observation derives its
/// `max_usage_bytes` from this one value instead of repeating the number.
pub const MAX_RETAINED_USAGE_BYTES: usize = 64 * 1024;

/// Byte bound the durable schema itself declares for `provider_usage.raw_usage_json` in migration
/// `0002_provider_observability.sql`.
const DURABLE_USAGE_COLUMN_BOUND: usize = 65_536;

/// A retained usage object above the durable column bound would be rejected by the schema and roll
/// back the whole provider batch, so the exported bound is checked while compiling.
const _: () = assert!(MAX_RETAINED_USAGE_BYTES <= DURABLE_USAGE_COLUMN_BOUND);

/// Bounded semantic parser limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ObservationLimits {
    /// Maximum bytes accepted for one request or non-stream response.
    pub max_semantic_bytes: usize,
    /// Maximum isolated usage object bytes, never above [`MAX_RETAINED_USAGE_BYTES`] for a caller
    /// that persists the observation.
    pub max_usage_bytes: usize,
    /// Maximum bytes in one SSE event.
    pub max_sse_event_bytes: usize,
    /// Maximum number of SSE events.
    pub max_sse_events: usize,
    /// Maximum JSON nesting depth.
    pub max_json_depth: usize,
    /// Maximum JSON array/object members inspected.
    pub max_json_items: usize,
    /// Maximum retained scalar string bytes.
    pub max_string_bytes: usize,
}

impl Default for ObservationLimits {
    fn default() -> Self {
        Self {
            max_semantic_bytes: 8 * 1024 * 1024,
            max_usage_bytes: MAX_RETAINED_USAGE_BYTES,
            max_sse_event_bytes: 1024 * 1024,
            max_sse_events: 100_000,
            max_json_depth: 64,
            max_json_items: 100_000,
            max_string_bytes: 16 * 1024,
        }
    }
}
/// Cohesive values used to construct validated observation limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::exhaustive_structs,
    reason = "Every bound is deliberately required at the public validation boundary."
)]
pub struct ObservationLimitValues {
    /// Maximum bytes accepted for one request or non-stream response.
    pub max_semantic_bytes: usize,
    /// Maximum isolated usage object bytes.
    pub max_usage_bytes: usize,
    /// Maximum bytes in one SSE event.
    pub max_sse_event_bytes: usize,
    /// Maximum number of SSE events.
    pub max_sse_events: usize,
    /// Maximum JSON nesting depth.
    pub max_json_depth: usize,
    /// Maximum JSON array/object members inspected.
    pub max_json_items: usize,
    /// Maximum retained scalar string bytes.
    pub max_string_bytes: usize,
}

impl From<ObservationLimits> for ObservationLimitValues {
    fn from(limits: ObservationLimits) -> Self {
        Self {
            max_semantic_bytes: limits.max_semantic_bytes,
            max_usage_bytes: limits.max_usage_bytes,
            max_sse_event_bytes: limits.max_sse_event_bytes,
            max_sse_events: limits.max_sse_events,
            max_json_depth: limits.max_json_depth,
            max_json_items: limits.max_json_items,
            max_string_bytes: limits.max_string_bytes,
        }
    }
}

impl ObservationLimits {
    /// Creates limits, rejecting zero values that would make parsing ambiguous.
    ///
    /// # Errors
    ///
    /// Returns [`LimitsError::Zero`] when any configured bound is zero.
    pub fn new(values: ObservationLimitValues) -> Result<Self, LimitsError> {
        if [
            values.max_semantic_bytes,
            values.max_usage_bytes,
            values.max_sse_event_bytes,
            values.max_sse_events,
            values.max_json_depth,
            values.max_json_items,
            values.max_string_bytes,
        ]
        .contains(&0)
        {
            return Err(LimitsError::Zero);
        }
        Ok(Self {
            max_semantic_bytes: values.max_semantic_bytes,
            max_usage_bytes: values.max_usage_bytes,
            max_sse_event_bytes: values.max_sse_event_bytes,
            max_sse_events: values.max_sse_events,
            max_json_depth: values.max_json_depth,
            max_json_items: values.max_json_items,
            max_string_bytes: values.max_string_bytes,
        })
    }
}

/// Invalid semantic limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum LimitsError {
    /// A semantic bound must be non-zero.
    #[error("semantic observation limits must be non-zero")]
    Zero,
}

/// Bounded bytes presented to an offline observer.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ObservationInput<'a> {
    /// Original provider bytes; never reserialized by the observer.
    pub bytes: &'a [u8],
    /// Finite parser limits.
    pub limits: ObservationLimits,
    /// Explicit content policy, defaulting to metadata only.
    pub content_capture: ContentCaptureMode,
}

impl<'a> ObservationInput<'a> {
    /// Creates metadata-only bounded input.
    #[must_use]
    pub const fn new(bytes: &'a [u8], limits: ObservationLimits) -> Self {
        Self {
            bytes,
            limits,
            content_capture: ContentCaptureMode::MetadataOnly,
        }
    }

    /// Sets the explicit content-capture capability.
    #[must_use]
    pub const fn with_content_capture(mut self, mode: ContentCaptureMode) -> Self {
        self.content_capture = mode;
        self
    }
}

/// Shared parser error. Protocol parsers normally return a partial observation instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ObservationError {
    /// Input exceeded its configured semantic bound.
    #[error("observation input exceeded the configured resource limit")]
    ResourceLimit,
    /// Input was not valid UTF-8 or contained a forbidden NUL byte.
    #[error("observation input is not valid provider text")]
    InvalidText,
    /// Input contained malformed framing or JSON.
    #[error("observation input is malformed")]
    Malformed,
}

/// Maps one parser failure to the observation status a fail-open observation reports.
pub(crate) const fn status_for_error(error: ObservationError) -> ObservationStatus {
    match error {
        ObservationError::ResourceLimit => ObservationStatus::ResourceLimit,
        ObservationError::InvalidText | ObservationError::Malformed => ObservationStatus::Malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::{DURABLE_USAGE_COLUMN_BOUND, ObservationLimits};

    #[test]
    fn the_published_default_usage_bound_never_exceeds_the_durable_column_bound() {
        assert!(
            ObservationLimits::default().max_usage_bytes <= DURABLE_USAGE_COLUMN_BOUND,
            "the published default must not retain a usage object the durable column rejects"
        );
    }
}
