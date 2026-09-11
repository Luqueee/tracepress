//! Canonical shadow context analysis domain values.

use std::{fmt, marker::PhantomData};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use tracepress_core::{
    ContextBlockOccurrenceId, ContextSnapshotId, OperationId, RequestId, SessionId,
};
use tracepress_provider::{ProviderKind, ProviderProtocol};

use crate::{BoundedMetadataText, ContextDigest, ContextVisibility};

macro_rules! define_extensible_enums {
    ($(
        #[doc = $enum_doc:literal]
        $name:ident($expecting:literal) {
            $(#[doc = $member_doc:literal] $member:ident = $wire:literal,)+
        }
    )+) => {
        $(
            #[doc = $enum_doc]
            ///
            /// A wire value this analysis version does not recognize deserializes to `Unknown`
            /// with its span and byte count intact, so a new provider item type can neither abort
            /// an analysis nor fail a durable read.
            #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
            #[non_exhaustive]
            pub enum $name {
                $(#[doc = $member_doc] $member,)+
                /// A value this analysis version does not recognize.
                Unknown,
            }

            impl $name {
                /// Returns the stable wire and durable name of this member.
                #[must_use]
                pub const fn as_wire_str(self) -> &'static str {
                    match self {
                        $(Self::$member => $wire,)+
                        Self::Unknown => "unknown",
                    }
                }
            }

            impl FromWireStr for $name {
                const EXPECTING: &'static str = $expecting;

                fn from_wire_str(value: &str) -> Self {
                    match value {
                        $($wire => Self::$member,)+
                        _ => Self::Unknown,
                    }
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str(self.as_wire_str())
                }
            }

            impl Serialize for $name {
                fn serialize<SerializerType>(
                    &self,
                    serializer: SerializerType,
                ) -> Result<SerializerType::Ok, SerializerType::Error>
                where
                    SerializerType: Serializer,
                {
                    serializer.serialize_str(self.as_wire_str())
                }
            }

            impl<'de> Deserialize<'de> for $name {
                fn deserialize<DeserializerType>(
                    deserializer: DeserializerType,
                ) -> Result<Self, DeserializerType::Error>
                where
                    DeserializerType: Deserializer<'de>,
                {
                    deserializer.deserialize_str(WireStrVisitor::<Self>::new())
                }
            }
        )+
    };
}

/// Version of the context analysis rules recorded with every snapshot.
///
/// It is persisted rather than assumed so a snapshot produced by a future re-analysis is
/// distinguishable instead of silently mixed with this one.
pub const CONTEXT_ANALYSIS_VERSION: u32 = 1;

/// The only provider whose requests this analysis version decomposes.
pub const ANALYZED_PROVIDER_KIND: ProviderKind = ProviderKind::OpenAi;

/// The only provider protocol this analysis version decomposes.
///
/// Any other observed protocol is recorded as
/// [`ContextAnalysisStatus::Unsupported`] instead of being decomposed by rules written for a
/// different document shape.
pub const ANALYZED_PROVIDER_PROTOCOL: ProviderProtocol = ProviderProtocol::OpenAiResponsesV1;

/// Outcome of one context analysis.
///
/// A partial analysis carrying useful aggregates stays [`ContextAnalysisStatus::Partial`]: none of
/// these outcomes may alter forwarded bytes, and none of them may be reported as complete.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextAnalysisStatus {
    /// The whole request was decomposed and the snapshot was finalized.
    Complete,
    /// Some blocks or aggregates could not be produced.
    Partial,
    /// A configured analysis bound was reached.
    ResourceLimit,
    /// The request bytes were not valid for the analyzed protocol.
    Malformed,
    /// Analysis was dropped because its bounded sink was full or closed.
    ObserverBackpressure,
    /// Correlation degradation prevented the analysis from being attributed.
    CorrelationDegraded,
    /// The observed route or protocol is not context-analyzed.
    Unsupported,
    /// Analysis was cancelled before completion.
    Cancelled,
}

/// Why a context analysis was not run or could not be handed to its sink.
///
/// A dropped analysis carries this compact reason instead of fabricating a digest, status, or
/// block list. The reason is also the wire value used by the CLI/daemon drop accounting.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextAnalysisDropReason {
    /// The proxy's non-blocking analysis permit or a bounded downstream queue was unavailable.
    ObserverBackpressure,
    /// A resource bound rejected the analysis before a result could be produced.
    ResourceLimit,
    /// The request could not be interpreted by the context analyzer.
    Malformed,
    /// Correlation degradation made attribution unsafe.
    CorrelationDegraded,
    /// The route or protocol is not supported by this analyzer.
    Unsupported,
    /// Analysis was cancelled before a result was handed to the sink.
    Cancelled,
}

impl ContextAnalysisDropReason {
    /// Returns the stable wire and durable name of this reason.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::ObserverBackpressure => "observer_backpressure",
            Self::ResourceLimit => "resource_limit",
            Self::Malformed => "malformed",
            Self::CorrelationDegraded => "correlation_degraded",
            Self::Unsupported => "unsupported",
            Self::Cancelled => "cancelled",
        }
    }
}

define_extensible_enums! {
    #[doc = "Canonical kind of one context block."]
    ContextBlockKind("a context block kind") {
        #[doc = "Top-level instructions."]
        Instructions = "instructions",
        #[doc = "A message item."]
        Message = "message",
        #[doc = "A text content part."]
        Text = "text",
        #[doc = "A reference to an image."]
        ImageReference = "image_reference",
        #[doc = "A reference to a file."]
        FileReference = "file_reference",
        #[doc = "A tool definition, including a provider built-in tool."]
        ToolDefinition = "tool_definition",
        #[doc = "A tool or function call."]
        ToolCall = "tool_call",
        #[doc = "A tool or function call output."]
        ToolResult = "tool_result",
        #[doc = "A reference to a provider-held item."]
        ItemReference = "item_reference",
        #[doc = "A reference to a provider-stored prompt."]
        PromptReference = "prompt_reference",
        #[doc = "A reference to provider-held response or conversation state."]
        ProviderStateReference = "provider_state_reference",
        #[doc = "Previous assistant output replayed in the request."]
        AssistantHistory = "assistant_history",
        #[doc = "A reasoning item whose content cannot be read."]
        OpaqueReasoning = "opaque_reasoning",
        #[doc = "An item whose content cannot be read."]
        Opaque = "opaque",
    }

    #[doc = "Conversational role of one context block, orthogonal to its kind."]
    ContextRole("a context role") {
        #[doc = "System role."]
        System = "system",
        #[doc = "Developer role."]
        Developer = "developer",
        #[doc = "User role."]
        User = "user",
        #[doc = "Assistant role."]
        Assistant = "assistant",
        #[doc = "Tool role."]
        Tool = "tool",
    }

    #[doc = "Structurally assigned provenance of one context block."]
    ContextOrigin("a context origin") {
        #[doc = "Authored by a human, such as a user message."]
        HumanAuthored = "human_authored",
        #[doc = "Produced by the agent, such as an assistant message or a tool call."]
        AgentGenerated = "agent_generated",
        #[doc = "Produced by a tool, such as a function call output."]
        ToolGenerated = "tool_generated",
        #[doc = "Declared as a tool schema."]
        ToolSchema = "tool_schema",
        #[doc = "Held by the provider rather than sent explicitly."]
        ProviderManaged = "provider_managed",
        #[doc = "Behind an external reference this phase never fetches."]
        ExternalReference = "external_reference",
        #[doc = "Produced by Tracepress itself."]
        TracepressGenerated = "tracepress_generated",
    }

    #[doc = "Content shape a deterministic structural detector recognized."]
    DetectedContentKind("a detected content kind") {
        #[doc = "One JSON document."]
        Json = "json",
        #[doc = "Newline-delimited JSON."]
        Ndjson = "ndjson",
        #[doc = "Timestamped or levelled log lines."]
        Log = "log",
        #[doc = "Path and line search results."]
        SearchResults = "search_results",
        #[doc = "Test-runner output."]
        TestResults = "test_results",
        #[doc = "Source code."]
        SourceCode = "source_code",
        #[doc = "A unified diff."]
        Diff = "diff",
        #[doc = "Prose or otherwise unstructured text."]
        PlainText = "plain_text",
        #[doc = "Content that does not decode as text."]
        BinaryLike = "binary_like",
    }
}

/// How a token count was produced.
///
/// No member means exact. `ModelMapped` and `GenericTokenizer` are deliberately unimplemented in
/// this phase rather than approximated by a heuristic wearing their name.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EstimateConfidence {
    /// Produced by a tokenizer mapped to the requested model.
    ModelMapped,
    /// Produced by a real tokenizer without a model mapping.
    GenericTokenizer,
    /// Produced by a deterministic structural heuristic.
    Heuristic,
}

/// Ordinal confidence of one structural detection.
///
/// A three-level scale keeps a deterministic heuristic from implying numeric precision it does
/// not have; a detector that cannot commit reports [`DetectedContentKind::Unknown`] with
/// [`DetectionConfidence::Low`] instead of guessing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DetectionConfidence {
    /// Weak or abstaining signal.
    Low,
    /// Multiple consistent structural signals.
    Medium,
    /// An unambiguous structural signal, such as a complete JSON parse.
    High,
}

/// Locally estimated tokens for one block of content.
///
/// A local estimate is never summed with provider-observed usage: the two are distinct concepts
/// stored in distinct columns.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct TokenEstimate {
    /// Estimated tokens. An estimate that cannot be produced is absent, never zero.
    pub tokens: u64,
    /// Identity of the estimator that produced the value.
    pub estimator: BoundedMetadataText,
    /// Version of that estimator's rules.
    pub estimator_version: u32,
    /// Encoding the estimator used, absent while no model-to-encoding mapping exists.
    pub encoding: Option<BoundedMetadataText>,
    /// How the value was produced.
    pub confidence: EstimateConfidence,
}

/// Outcome of one deterministic content detection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct DetectionResult {
    /// Recognized content shape.
    pub kind: DetectedContentKind,
    /// Ordinal confidence in that shape.
    pub confidence: DetectionConfidence,
    /// Version of the detector rules.
    pub detector_version: u32,
}

/// A versioned fingerprint of decoded semantic content.
///
/// The version is stored with the digest because an unversioned semantic fingerprint may never be
/// used for an equivalence claim or for training.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SemanticFingerprint {
    /// Version of the semantic fingerprint rules.
    pub fingerprint_version: u32,
    /// Digest over the fingerprint version, block kind, role, and decoded content.
    pub digest: ContextDigest,
}

/// Where one block's bytes are inside the original request.
///
/// The raw span is the authoritative structural identity within a request: duplicate object keys
/// make a path ambiguous, so `semantic_path` is a bounded convenience and is never treated as
/// unique.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct BlockLocator {
    /// JSON-Pointer-shaped path to the block, bounded and not unique.
    pub semantic_path: BoundedMetadataText,
    /// First byte of the JSON value, including an enclosing quote, brace, or bracket.
    pub raw_value_start: u64,
    /// One past the last byte of the JSON value.
    pub raw_value_end: u64,
    /// Index of this member among colliding members of the same path.
    pub occurrence: u32,
}

/// One typed, positioned, measured block of an observed request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ContextBlockOccurrence {
    /// Daemon-allocated identity of this block.
    pub id: ContextBlockOccurrenceId,
    /// Snapshot this block belongs to.
    pub snapshot_id: ContextSnapshotId,
    /// Document order of the block within its snapshot, stable for identical bytes.
    pub ordinal: u32,
    /// Enclosing block, absent for a top-level block.
    pub parent_id: Option<ContextBlockOccurrenceId>,
    /// Canonical kind of the block.
    pub kind: ContextBlockKind,
    /// Conversational role of the block.
    pub role: ContextRole,
    /// Structurally assigned provenance of the block.
    pub origin: ContextOrigin,
    /// Position of the block inside the original request bytes.
    pub locator: BlockLocator,
    /// Bytes the block occupies in the original request.
    pub raw_bytes: u64,
    /// Digest of the raw span bytes, detecting byte-identical repetition.
    pub exact_fingerprint: ContextDigest,
    /// Versioned digest of decoded content, absent where equivalence is not definable.
    pub semantic_fingerprint: Option<SemanticFingerprint>,
    /// Locally estimated tokens, absent where no estimate applies.
    pub token_estimate: Option<TokenEstimate>,
    /// Structural detection result, absent where no detector ran.
    pub detector_result: Option<DetectionResult>,
}

/// One shadow context analysis of one observed provider request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ContextSnapshot {
    /// Daemon-allocated identity of this snapshot.
    pub id: ContextSnapshotId,
    /// Session the analyzed request belongs to.
    pub session_id: SessionId,
    /// Provider request the analysis observed.
    pub provider_request_id: RequestId,
    /// Inference operation the request served.
    pub inference_operation_id: OperationId,
    /// Version of the analysis rules that produced this snapshot.
    pub analysis_version: u32,
    /// Outcome of the analysis.
    pub status: ContextAnalysisStatus,
    /// Number of fully delimited blocks whose semantic type this version did not recognize.
    pub unknown_block_count: u32,
    /// Recognized-block coverage in basis points (`10_000` = 100%), when the denominator exists.
    pub semantic_coverage_basis_points: Option<u16>,
    /// Per-signal visibility of the analyzed request.
    pub visibility: ContextVisibility,
    /// Digest over the exact JSON bytes supplied to the analyzer.
    ///
    /// A request's separately recorded wire digest identifies the forwarded representation when
    /// analysis-only decoding was required; this digest never claims to be a digest of that wire
    /// representation.
    pub request_content_hash: ContextDigest,
    /// Whether a duplicate object key was observed anywhere in the request.
    pub duplicate_key_detected: bool,
    /// Explicit blocks recorded for this snapshot.
    pub explicit_block_count: u32,
    /// Request bytes analysis inspected.
    pub analyzed_bytes: u64,
    /// Request bytes analysis did not inspect, forwarded in full regardless.
    pub skipped_bytes: u64,
    /// Microsecond timestamp at which analysis started.
    pub started_at_us: u64,
    /// Microsecond timestamp at which the snapshot was finalized, absent until it commits.
    pub completed_at_us: Option<u64>,
}

/// Maps one wire value to a member, tolerating a value this version does not know.
trait FromWireStr: Sized {
    /// Human-readable description of the accepted values.
    const EXPECTING: &'static str;

    /// Returns the member named by `value`, or the unknown member.
    fn from_wire_str(value: &str) -> Self;
}

/// Accepts a wire value without allocating an intermediate string.
struct WireStrVisitor<MemberType>(PhantomData<fn() -> MemberType>);

impl<MemberType> WireStrVisitor<MemberType> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<MemberType> de::Visitor<'_> for WireStrVisitor<MemberType>
where
    MemberType: FromWireStr,
{
    type Value = MemberType;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(MemberType::EXPECTING)
    }

    fn visit_str<ErrorType>(self, value: &str) -> Result<Self::Value, ErrorType>
    where
        ErrorType: de::Error,
    {
        Ok(MemberType::from_wire_str(value))
    }
}

#[cfg(test)]
mod tests {
    use tracepress_core::{
        ContextBlockOccurrenceId, ContextSnapshotId, OperationId, RequestId, SessionId,
        UuidV7Generator,
    };

    use super::{
        BlockLocator, BoundedMetadataText, CONTEXT_ANALYSIS_VERSION, ContextAnalysisStatus,
        ContextBlockKind, ContextBlockOccurrence, ContextDigest, ContextOrigin, ContextRole,
        ContextSnapshot, ContextVisibility, DetectedContentKind, DetectionConfidence,
        DetectionResult, EstimateConfidence, SemanticFingerprint, TokenEstimate,
    };

    #[test]
    fn a_block_occurrence_survives_the_wire() -> Result<(), serde_json::Error> {
        let block = sample_block();

        let decoded =
            serde_json::from_str::<ContextBlockOccurrence>(&serde_json::to_string(&block)?)?;

        assert_eq!(decoded, block);
        Ok(())
    }

    #[test]
    fn a_snapshot_survives_the_wire() -> Result<(), serde_json::Error> {
        let generator = UuidV7Generator::new();
        let snapshot = ContextSnapshot {
            id: ContextSnapshotId::generate(&generator),
            session_id: SessionId::generate(&generator),
            provider_request_id: RequestId::generate(&generator),
            inference_operation_id: OperationId::generate(&generator),
            analysis_version: CONTEXT_ANALYSIS_VERSION,
            status: ContextAnalysisStatus::Partial,
            unknown_block_count: 0,
            semantic_coverage_basis_points: None,
            visibility: ContextVisibility {
                explicit_request_complete: false,
                uses_previous_response: true,
                uses_conversation_state: false,
                uses_item_references: false,
                uses_prompt_reference: false,
                uses_external_files: false,
                uses_external_images: false,
                contains_opaque_items: false,
            },
            request_content_hash: ContextDigest::from_bytes(b"{\"model\":\"gpt\"}"),
            duplicate_key_detected: true,
            explicit_block_count: 12,
            analyzed_bytes: 4_096,
            skipped_bytes: 128,
            started_at_us: 1_700_000_000_000_000,
            completed_at_us: None,
        };

        let decoded = serde_json::from_str::<ContextSnapshot>(&serde_json::to_string(&snapshot)?)?;

        assert_eq!(decoded, snapshot);
        Ok(())
    }

    #[test]
    fn a_block_debug_reports_structure_without_the_metadata_value() {
        let block = sample_block();

        let rendered = format!("{block:?}");

        assert!(!rendered.contains("customer_email"), "{rendered}");
        assert!(rendered.contains("ToolResult"), "{rendered}");
        assert!(rendered.contains("raw_value_start"), "{rendered}");
    }

    fn sample_block() -> ContextBlockOccurrence {
        let generator = UuidV7Generator::new();
        ContextBlockOccurrence {
            id: ContextBlockOccurrenceId::generate(&generator),
            snapshot_id: ContextSnapshotId::generate(&generator),
            ordinal: 7,
            parent_id: Some(ContextBlockOccurrenceId::generate(&generator)),
            kind: ContextBlockKind::ToolResult,
            role: ContextRole::Tool,
            origin: ContextOrigin::ToolGenerated,
            locator: BlockLocator {
                semantic_path: BoundedMetadataText::semantic_path("/input/2/customer_email"),
                raw_value_start: 128,
                raw_value_end: 512,
                occurrence: 0,
            },
            raw_bytes: 384,
            exact_fingerprint: ContextDigest::from_bytes(b"tool result bytes"),
            semantic_fingerprint: Some(SemanticFingerprint {
                fingerprint_version: 1,
                digest: ContextDigest::from_bytes(b"decoded tool result"),
            }),
            token_estimate: Some(TokenEstimate {
                tokens: 96,
                estimator: BoundedMetadataText::estimator_identity("structural-heuristic"),
                estimator_version: 1,
                encoding: None,
                confidence: EstimateConfidence::Heuristic,
            }),
            detector_result: Some(DetectionResult {
                kind: DetectedContentKind::Json,
                confidence: DetectionConfidence::High,
                detector_version: 1,
            }),
        }
    }
}
