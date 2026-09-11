//! Safe, allowlisted parsing of `OpenAI` Responses request metadata.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domain::{
    AnalysisDecodeStatus, CompactionProtocol, CompactionTrigger, ContentEncoding,
    OPENAI_RESPONSES_PARSER_VERSION, ObservationInput, ObservationStatus, ProviderKind,
    ProviderProtocol, ProviderRequestKind, ProviderTransport, status_for_error,
};
use crate::json::{self, NestedField, StringExtraction};

/// Request metadata extracted without retaining prompt or tool content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RequestObservation {
    /// Canonical provider identity.
    pub provider: ProviderKind,
    /// Canonical protocol identity.
    pub protocol: ProviderProtocol,
    /// Transport surface on which this request was forwarded.
    #[serde(default)]
    pub transport: ProviderTransport,
    /// Whether this is a normal turn or a provider compaction request.
    #[serde(default)]
    pub request_kind: ProviderRequestKind,
    /// Trigger classification found on a V2 compaction request, never the trigger payload.
    #[serde(default)]
    pub compaction_trigger: Option<CompactionTrigger>,
    /// Version of the selected endpoint profile.
    #[serde(default)]
    pub endpoint_profile_version: Option<u32>,
    /// Parser version used for this observation.
    pub parser_version: u32,
    /// Semantic outcome.
    pub status: ObservationStatus,
    /// Exact length of the bounded request body presented to the parser.
    pub request_bytes: Option<u64>,
    /// Exact bytes received and forwarded on the wire.
    #[serde(default)]
    pub wire_bytes: Option<u64>,
    /// SHA-256 over the exact wire body, distinct from the context-analysis digest.
    #[serde(default)]
    pub wire_sha256: Option<Box<[u8]>>,
    /// Bytes made available to the analyzer after analysis-only decoding.
    #[serde(default)]
    pub decoded_bytes: Option<u64>,
    /// Content encoding observed on the wire.
    #[serde(default)]
    pub content_encoding: ContentEncoding,
    /// Outcome of analysis-only decoding.
    #[serde(default)]
    pub analysis_decode_status: AnalysisDecodeStatus,
    /// Bounded decoder duration in microseconds.
    #[serde(default)]
    pub decode_duration_us: Option<u64>,
    /// Version of the analysis decoder.
    #[serde(default)]
    pub decoder_version: Option<u32>,
    /// Requested model name.
    pub model: Option<String>,
    /// Whether streaming was requested.
    pub stream: Option<bool>,
    /// Whether background processing was requested.
    pub background: Option<bool>,
    /// Whether provider storage was requested.
    pub store: Option<bool>,
    /// Requested reasoning effort.
    pub reasoning_effort: Option<String>,
    /// Requested output verbosity.
    pub verbosity: Option<String>,
    /// Requested truncation behavior.
    pub truncation: Option<String>,
    /// Whether a previous response ID was supplied (the value is never retained).
    pub has_previous_response_id: bool,
    /// Number of top-level input items.
    pub input_item_count: Option<u64>,
    /// Number of configured tools.
    pub tool_count: Option<u64>,
    /// Number of text input blocks.
    pub text_input_blocks: Option<u64>,
    /// Number of image input blocks.
    pub image_input_blocks: Option<u64>,
    /// Number of file input blocks.
    pub file_input_blocks: Option<u64>,
}

/// Compatibility name spelling out the protocol represented by this observation.
pub type OpenAiResponsesRequestObservation = RequestObservation;

impl RequestObservation {
    pub(crate) const fn empty(status: ObservationStatus) -> Self {
        Self {
            provider: ProviderKind::OpenAi,
            protocol: ProviderProtocol::OpenAiResponsesV1,
            transport: ProviderTransport::OpenAiPublicApi,
            request_kind: ProviderRequestKind::Turn,
            compaction_trigger: None,
            endpoint_profile_version: None,
            parser_version: OPENAI_RESPONSES_PARSER_VERSION,
            status,
            request_bytes: None,
            wire_bytes: None,
            wire_sha256: None,
            decoded_bytes: None,
            content_encoding: ContentEncoding::Identity,
            analysis_decode_status: AnalysisDecodeStatus::Identity,
            decode_duration_us: None,
            decoder_version: None,
            model: None,
            stream: None,
            background: None,
            store: None,
            reasoning_effort: None,
            verbosity: None,
            truncation: None,
            has_previous_response_id: false,
            input_item_count: None,
            tool_count: None,
            text_input_blocks: None,
            image_input_blocks: None,
            file_input_blocks: None,
        }
    }

    /// Constructs an observation whose semantic body was unavailable to the parser.
    #[must_use]
    pub const fn unavailable(status: ObservationStatus) -> Self {
        Self::empty(status)
    }

    /// Constructs a metadata-only observation for a request that is not parsed as Responses JSON.
    #[allow(
        clippy::too_many_arguments,
        reason = "the transport-only record keeps each bounded wire metric explicit"
    )]
    #[must_use]
    pub const fn transport_only(
        request_kind: ProviderRequestKind,
        request_bytes: u64,
        wire_bytes: u64,
        content_encoding: ContentEncoding,
        transport: ProviderTransport,
        endpoint_profile_version: u32,
    ) -> Self {
        let mut result = Self::empty(ObservationStatus::Unsupported);
        result.request_bytes = Some(request_bytes);
        result.wire_bytes = Some(wire_bytes);
        result.content_encoding = content_encoding;
        result.transport = transport;
        result.endpoint_profile_version = Some(endpoint_profile_version);
        result.request_kind = request_kind;
        result.compaction_trigger = request_kind.compaction_trigger();
        result
    }
}

/// Parses one bounded, non-streaming Responses v1 request body.
#[must_use]
pub fn parse_request(input: ObservationInput<'_>) -> RequestObservation {
    let mut result = RequestObservation::empty(ObservationStatus::Complete);
    result.request_bytes = u64::try_from(input.bytes.len()).ok();
    result.wire_bytes = result.request_bytes;
    result.decoded_bytes = result.request_bytes;
    let parsed =
        json::validate_text(input).and_then(|()| json::parse_bounded(input.bytes, input.limits));
    let value = match parsed {
        Ok(value) => value,
        Err(error) => {
            result.status = status_for_error(error);
            return result;
        }
    };
    let Some(object) = value.as_object() else {
        result.status = ObservationStatus::Malformed;
        return result;
    };
    let mut extraction = StringExtraction::new(input.limits.max_string_bytes);

    result.model = extraction.extract(object, "model");
    result.stream = optional_bool(object, "stream", &mut extraction.partial);
    result.background = optional_bool(object, "background", &mut extraction.partial);
    result.store = optional_bool(object, "store", &mut extraction.partial);
    result.reasoning_effort =
        extraction.extract_nested(object, NestedField::new("reasoning", "effort"));
    result.verbosity = extraction.extract_nested(object, NestedField::new("text", "verbosity"));
    result.truncation = extraction.extract(object, "truncation");
    result.has_previous_response_id = match object.get("previous_response_id") {
        None | Some(Value::Null) => false,
        Some(Value::String(_)) => true,
        Some(_) => {
            extraction.partial = true;
            false
        }
    };

    result.input_item_count = array_count(object, "input", &mut extraction.partial);
    result.tool_count = array_count(object, "tools", &mut extraction.partial);
    if let Some(input_items) = object.get("input").and_then(Value::as_array) {
        let mut counts = InputBlockCounts::default();
        for item in input_items {
            count_blocks(item, &mut counts, &mut extraction.partial);
        }
        if input_items.iter().any(is_compaction_trigger) {
            let trigger = CompactionTrigger::Unknown;
            result.request_kind = ProviderRequestKind::Compaction {
                protocol: CompactionProtocol::ResponsesTriggerV2,
                trigger,
            };
            result.compaction_trigger = Some(trigger);
        }
        result.text_input_blocks = Some(counts.text);
        result.image_input_blocks = Some(counts.image);
        result.file_input_blocks = Some(counts.file);
    }
    if extraction.partial {
        result.status = ObservationStatus::Partial;
    }
    result
}

fn is_compaction_trigger(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
        == Some("compaction_trigger")
}

fn optional_bool(map: &Map<String, Value>, key: &str, partial: &mut bool) -> Option<bool> {
    match map.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::Bool(value)) => Some(*value),
        Some(_) => {
            *partial = true;
            None
        }
    }
}

fn array_count(map: &Map<String, Value>, key: &str, partial: &mut bool) -> Option<u64> {
    match map.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::Array(values)) => u64::try_from(values.len()).ok(),
        Some(_) => {
            *partial = true;
            None
        }
    }
}

#[derive(Default)]
struct InputBlockCounts {
    text: u64,
    image: u64,
    file: u64,
}

fn count_blocks(value: &Value, counts: &mut InputBlockCounts, partial: &mut bool) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(content) = object.get("content") else {
        return;
    };
    let Some(blocks) = content.as_array() else {
        *partial = true;
        return;
    };
    for block in blocks {
        let Some(kind) = block.get("type").and_then(Value::as_str) else {
            continue;
        };
        let counter = match kind {
            "input_text" | "text" => &mut counts.text,
            "input_image" | "image_url" | "image" => &mut counts.image,
            "input_file" | "file" => &mut counts.file,
            _ => {
                *partial = true;
                continue;
            }
        };
        if let Some(next) = counter.checked_add(1) {
            *counter = next;
        } else {
            *partial = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObservationLimitValues, ObservationLimits};

    #[test]
    fn request_allowlist_excludes_content_and_counts_blocks() {
        let body = br#"{"model":"gpt-5.6-sol","input":[{"role":"user","content":[{"type":"input_text","text":"secret"},{"type":"input_image","image_url":"https://secret"}]}],"tools":[{"type":"function","name":"private"}],"reasoning":{"effort":"medium"},"text":{"verbosity":"low"},"stream":true,"metadata":{"secret":"value"},"user":"person","previous_response_id":"resp_1"}"#;
        let observation = parse_request(ObservationInput::new(body, ObservationLimits::default()));
        assert_eq!(observation.status, ObservationStatus::Complete);
        assert_eq!(observation.input_item_count, Some(1));
        assert_eq!(observation.tool_count, Some(1));
        assert_eq!(observation.text_input_blocks, Some(1));
        assert_eq!(observation.image_input_blocks, Some(1));
        assert_eq!(observation.model.as_deref(), Some("gpt-5.6-sol"));
        assert!(observation.has_previous_response_id);
    }

    #[test]
    fn an_oversized_retained_string_is_dropped_without_losing_the_request() {
        let limits = tight_limits();
        let observation = parse_request(ObservationInput::new(
            br#"{"model":"gpt-5","stream":true,"tools":[{"type":"function","name":"t"}]}"#,
            limits,
        ));
        assert_eq!(observation.status, ObservationStatus::Partial);
        assert_eq!(observation.model, None);
        assert_eq!(observation.stream, Some(true));
        assert_eq!(observation.tool_count, Some(1));
    }

    #[test]
    fn a_long_string_the_parser_never_retains_keeps_the_observation_complete() {
        let observation = parse_request(ObservationInput::new(
            br#"{"model":"gpt","input":[{"content":[{"type":"text","text":"secret"}]}]}"#,
            tight_limits(),
        ));
        assert_eq!(observation.status, ObservationStatus::Complete);
        assert_eq!(observation.input_item_count, Some(1));
        assert_eq!(observation.text_input_blocks, Some(1));
        assert_eq!(observation.model.as_deref(), Some("gpt"));
    }

    #[test]
    fn malformed_and_oversized_bodies_are_fail_open() {
        let limits = tight_limits();
        let oversized_body = format!(r#"{{"model":"gpt","note":"{}"}}"#, "n".repeat(256));
        assert_eq!(
            parse_request(ObservationInput::new(oversized_body.as_bytes(), limits)).status,
            ObservationStatus::ResourceLimit
        );
        assert_eq!(
            parse_request(ObservationInput::new(br"{]", ObservationLimits::default())).status,
            ObservationStatus::Malformed
        );
        assert_eq!(
            parse_request(ObservationInput::new(
                br#"{"model":"a","model":"b"}"#,
                ObservationLimits::default()
            ))
            .status,
            ObservationStatus::Malformed
        );
    }

    #[test]
    fn an_escaped_nul_never_reaches_a_retained_field() {
        let observation = parse_request(ObservationInput::new(
            br#"{"model":"gpt\u0000hidden","stream":false}"#,
            ObservationLimits::default(),
        ));
        assert_eq!(observation.model, None);
        assert_eq!(observation.stream, Some(false));
        assert_eq!(observation.status, ObservationStatus::Partial);
    }

    #[test]
    fn compaction_trigger_classifies_the_responses_request_without_retaining_content() {
        let observation = parse_request(ObservationInput::new(
            br#"{"model":"gpt-5.6-sol","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"private"}]},{"type":"compaction_trigger"}]}"#,
            ObservationLimits::default(),
        ));

        assert_eq!(observation.status, ObservationStatus::Complete);
        assert_eq!(observation.request_kind.as_wire_str(), "compaction_v2");
        assert_eq!(
            observation.compaction_trigger,
            Some(CompactionTrigger::Unknown)
        );
    }

    fn tight_limits() -> ObservationLimits {
        ObservationLimits::new(ObservationLimitValues {
            max_semantic_bytes: 128,
            max_usage_bytes: 128,
            max_sse_event_bytes: 128,
            max_sse_events: 128,
            max_json_depth: 8,
            max_json_items: 128,
            max_string_bytes: 4,
        })
        .unwrap_or_default()
    }
}
