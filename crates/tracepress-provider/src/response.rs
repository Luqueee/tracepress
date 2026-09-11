#![allow(
    clippy::redundant_pub_crate,
    reason = "the streaming observer sibling module shares this usage interpretation"
)]

//! Offline `OpenAI` Responses v1 response and usage parsing.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::domain::{
    AnomalyFlags, NormalizedUsage, OPENAI_RESPONSES_PARSER_VERSION, ObservationInput,
    ObservationStatus, ProviderKind, ProviderProtocol, ProviderResponseState, RawProviderUsage,
    USAGE_NORMALIZER_VERSION, UsageStatus, status_for_error,
};
use crate::json::{self, StringExtraction};

/// Largest magnitude a durable signed 64-bit counter column can hold.
const MAX_DURABLE_MAGNITUDE: f64 = 9.223_372_036_854_776e18;

/// Semantic metadata from one non-streaming Responses response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ResponseObservation {
    /// Canonical provider identity.
    pub provider: ProviderKind,
    /// Canonical protocol identity.
    pub protocol: ProviderProtocol,
    /// Parser version used for this observation.
    pub parser_version: u32,
    /// Semantic outcome.
    pub status: ObservationStatus,
    /// Provider response identifier, when present.
    pub provider_response_id: Option<String>,
    /// Provider model identifier, when present.
    pub model: Option<String>,
    /// Provider creation timestamp exactly as the provider reported it.
    pub created_at: Option<u64>,
    /// Canonical provider lifecycle state.
    pub response_state: ProviderResponseState,
    /// Provider-provided incomplete reason.
    pub incomplete_reason: Option<String>,
    /// Provider-provided error code.
    pub error_code: Option<String>,
    /// Exact bounded usage object, if present and valid.
    pub raw_usage: Option<RawProviderUsage>,
    /// Whether a streamed or document response contained a compaction output item.
    #[serde(default)]
    pub compaction_output_seen: bool,
    /// Checked canonical usage values.
    pub normalized_usage: Option<NormalizedUsage>,
    /// Number of upstream chunks observed by a stream adapter.
    pub chunk_count: Option<u64>,
    /// Number of upstream bytes observed by a stream adapter.
    pub byte_count: Option<u64>,
    /// Usage availability.
    pub usage_status: UsageStatus,
    /// Microseconds from the start of observation to the first observed upstream byte.
    pub ttfb_us: Option<u64>,
    /// Microseconds from the start of observation to the first semantic output event.
    pub ttft_us: Option<u64>,
    /// Microseconds from the start of observation to its terminal decision.
    pub duration_us: Option<u64>,
}

impl ResponseObservation {
    pub(crate) const fn empty(status: ObservationStatus) -> Self {
        Self {
            provider: ProviderKind::OpenAi,
            protocol: ProviderProtocol::OpenAiResponsesV1,
            parser_version: OPENAI_RESPONSES_PARSER_VERSION,
            status,
            provider_response_id: None,
            model: None,
            created_at: None,
            response_state: ProviderResponseState::Unknown,
            incomplete_reason: None,
            error_code: None,
            raw_usage: None,
            compaction_output_seen: false,
            normalized_usage: None,
            chunk_count: None,
            byte_count: None,
            usage_status: UsageStatus::Unavailable,
            ttfb_us: None,
            ttft_us: None,
            duration_us: None,
        }
    }
}

/// Compatibility name spelling out the protocol represented by this observation.
pub type OpenAiResponsesResponseObservation = ResponseObservation;

/// Parses one bounded, non-streaming Responses v1 response body.
#[must_use]
pub fn parse_response(input: ObservationInput<'_>) -> ResponseObservation {
    let mut result = ResponseObservation::empty(ObservationStatus::Complete);
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
    result.provider_response_id = extraction.extract(object, "id");
    result.model = extraction.extract(object, "model");
    result.created_at = optional_u64(object.get("created_at"), &mut extraction.partial);
    result.response_state = match object.get("status") {
        None | Some(Value::Null) => ProviderResponseState::Unknown,
        Some(Value::String(status)) => map_status(status.as_str()),
        Some(_) => {
            extraction.partial = true;
            ProviderResponseState::Unknown
        }
    };
    if let Some(incomplete) = object.get("incomplete_details") {
        match incomplete {
            Value::Object(map) => result.incomplete_reason = extraction.extract(map, "reason"),
            Value::Null => {}
            _ => extraction.partial = true,
        }
    }
    if let Some(error) = object.get("error") {
        match error {
            Value::Object(map) => result.error_code = extraction.extract(map, "code"),
            Value::Null => {}
            _ => extraction.partial = true,
        }
    }
    let usage = extract_usage(object, input.bytes, input.limits.max_usage_bytes);
    result.usage_status =
        settle_usage_status(usage.status, is_terminal_state(result.response_state));
    result.normalized_usage = usage.normalized;
    result.raw_usage = usage.raw;
    if matches!(usage.outcome, UsageOutcome::Unusable) {
        extraction.partial = true;
    }
    if extraction.partial {
        result.status = ObservationStatus::Partial;
    }
    if matches!(usage.outcome, UsageOutcome::Oversized) {
        result.status = ObservationStatus::ResourceLimit;
    }
    result.compaction_output_seen = contains_compaction_item_object(object);
    result
}

/// Detects the provider's compaction output item without retaining its content.
pub(crate) fn contains_compaction_item(value: &Value) -> bool {
    match value {
        Value::Object(object) => contains_compaction_item_object(object),
        Value::Array(values) => values.iter().any(contains_compaction_item),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn contains_compaction_item_object(object: &Map<String, Value>) -> bool {
    (object.get("type").and_then(Value::as_str) == Some("compaction"))
        || object.values().any(contains_compaction_item)
}

/// What the top-level `usage` member of one provider response object yielded.
pub(crate) struct UsageEvidence {
    pub(crate) raw: Option<RawProviderUsage>,
    pub(crate) normalized: Option<NormalizedUsage>,
    pub(crate) status: UsageStatus,
    pub(crate) outcome: UsageOutcome,
}

/// Interpretation outcome of one `usage` member.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum UsageOutcome {
    /// No usage member was present.
    Absent,
    /// The provider's own usage object was retained exactly and normalized.
    Retained,
    /// A usage member was present but could not be interpreted.
    Unusable,
    /// The usage object exceeded its configured byte bound.
    Oversized,
}

/// Retains and normalizes the top-level `usage` member of one response object.
///
/// `object` is the parsed response object and `object_bytes` its exact serialized bytes; the
/// parsed value decides whether a usage object exists and the byte scan only recovers that
/// member's exact span, so a `usage` key nested anywhere inside another member is never retained.
pub(crate) fn extract_usage(
    object: &Map<String, Value>,
    object_bytes: &[u8],
    max_usage_bytes: usize,
) -> UsageEvidence {
    let unusable = |outcome: UsageOutcome, status: UsageStatus| UsageEvidence {
        raw: None,
        normalized: None,
        status,
        outcome,
    };
    match object.get("usage") {
        None | Some(Value::Null) => unusable(UsageOutcome::Absent, UsageStatus::Unavailable),
        Some(Value::Object(_)) => {
            let Some(bytes) = json::member_span(object_bytes, b"usage") else {
                return unusable(UsageOutcome::Unusable, UsageStatus::Partial);
            };
            match RawProviderUsage::new(bytes, max_usage_bytes) {
                Ok(raw) => {
                    let normalized = normalize_usage(&raw);
                    let status = normalized.status;
                    UsageEvidence {
                        raw: Some(raw),
                        normalized: Some(normalized),
                        status,
                        outcome: UsageOutcome::Retained,
                    }
                }
                Err(crate::UsageError::ResourceLimit) => {
                    unusable(UsageOutcome::Oversized, UsageStatus::Unavailable)
                }
                Err(crate::UsageError::Malformed) => {
                    unusable(UsageOutcome::Unusable, UsageStatus::Partial)
                }
            }
        }
        Some(_) => unusable(UsageOutcome::Unusable, UsageStatus::Partial),
    }
}

/// Downgrades usage that has not been confirmed by a terminal response.
pub(crate) const fn settle_usage_status(status: UsageStatus, terminal: bool) -> UsageStatus {
    match status {
        UsageStatus::Final if !terminal => UsageStatus::Partial,
        UsageStatus::Final | UsageStatus::Partial | UsageStatus::Unavailable => status,
    }
}

/// Whether a lifecycle state is terminal evidence about the provider's own work.
pub(crate) const fn is_terminal_state(state: ProviderResponseState) -> bool {
    match state {
        ProviderResponseState::Completed
        | ProviderResponseState::Incomplete
        | ProviderResponseState::Failed
        | ProviderResponseState::Cancelled => true,
        ProviderResponseState::Queued
        | ProviderResponseState::InProgress
        | ProviderResponseState::Disconnected
        | ProviderResponseState::Unknown => false,
    }
}

/// Normalizes an exact provider usage object with checked arithmetic.
#[must_use]
pub fn normalize_usage(raw: &RawProviderUsage) -> NormalizedUsage {
    let mut anomalies = AnomalyFlags::default();
    let Ok(value) = serde_json::from_slice::<Value>(raw.as_bytes()) else {
        anomalies.invalid_number = true;
        return unavailable_with_anomalies(anomalies);
    };
    let Some(object) = value.as_object() else {
        anomalies.invalid_number = true;
        return unavailable_with_anomalies(anomalies);
    };
    let input_total = number(object.get("input_tokens"), &mut anomalies);
    let input_cached = nested_number(
        object,
        NestedUsageField::new("input_tokens_details", "cached_tokens"),
        &mut anomalies,
    );
    let cache_write = nested_number(
        object,
        NestedUsageField::new("input_tokens_details", "cache_write_tokens"),
        &mut anomalies,
    )
    .or_else(|| {
        nested_number(
            object,
            NestedUsageField::new("input_tokens_details", "cache_write"),
            &mut anomalies,
        )
    });
    let output_total = number(object.get("output_tokens"), &mut anomalies);
    let output_reasoning = nested_number(
        object,
        NestedUsageField::new("output_tokens_details", "reasoning_tokens"),
        &mut anomalies,
    );
    let total = number(object.get("total_tokens"), &mut anomalies);
    let input_uncached = match (input_total, input_cached) {
        (Some(total_input), Some(cached)) if cached <= total_input => {
            total_input.checked_sub(cached)
        }
        (Some(_), Some(_)) => {
            anomalies.cached_exceeds_input = true;
            None
        }
        _ => None,
    };
    if let (Some(output), Some(reasoning)) = (output_total, output_reasoning) {
        if reasoning > output {
            anomalies.reasoning_exceeds_output = true;
        }
    }
    if let (Some(total_tokens), Some(input), Some(output)) = (total, input_total, output_total) {
        if input.checked_add(output) != Some(total_tokens) {
            anomalies.inconsistent_total = true;
        }
    }
    let reported = [
        input_total,
        input_cached,
        cache_write,
        output_total,
        output_reasoning,
        total,
    ]
    .iter()
    .any(Option::is_some);
    let status = if !reported {
        UsageStatus::Unavailable
    } else if anomalies.any() {
        UsageStatus::Partial
    } else {
        UsageStatus::Final
    };
    NormalizedUsage {
        input_total,
        input_cached,
        input_uncached,
        cache_write,
        output_total,
        output_reasoning,
        total,
        status,
        normalizer_version: USAGE_NORMALIZER_VERSION,
        anomalies,
    }
}

const fn unavailable_with_anomalies(anomalies: AnomalyFlags) -> NormalizedUsage {
    NormalizedUsage {
        input_total: None,
        input_cached: None,
        input_uncached: None,
        cache_write: None,
        output_total: None,
        output_reasoning: None,
        total: None,
        status: UsageStatus::Unavailable,
        normalizer_version: USAGE_NORMALIZER_VERSION,
        anomalies,
    }
}

fn number(value: Option<&Value>, anomalies: &mut AnomalyFlags) -> Option<u64> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => durable_count(number, anomalies),
        Some(_) => {
            anomalies.invalid_number = true;
            None
        }
    }
}

/// Accepts only a non-negative integer a durable signed 64-bit counter column can hold.
///
/// A magnitude beyond that range is an overflow and never becomes a component value, so no
/// provider-chosen number can make the durable write unsatisfiable. A wrongly typed, negative, or
/// fractional value is an invalid number instead; nothing exceeded 64 bits in that case.
fn durable_count(number: &Number, anomalies: &mut AnomalyFlags) -> Option<u64> {
    if let Some(value) = number.as_i64() {
        if value < 0 {
            anomalies.invalid_number = true;
            return None;
        }
        return Some(value.unsigned_abs());
    }
    if number.as_u64().is_some() {
        anomalies.overflow = true;
        return None;
    }
    match number.as_f64() {
        Some(value) if value.is_finite() && value.abs() <= MAX_DURABLE_MAGNITUDE => {
            anomalies.invalid_number = true;
        }
        Some(_) | None => anomalies.overflow = true,
    }
    None
}

#[derive(Clone, Copy)]
struct NestedUsageField {
    parent: &'static str,
    key: &'static str,
}

impl NestedUsageField {
    const fn new(parent: &'static str, key: &'static str) -> Self {
        Self { parent, key }
    }
}

fn nested_number(
    object: &Map<String, Value>,
    field: NestedUsageField,
    anomalies: &mut AnomalyFlags,
) -> Option<u64> {
    match object.get(field.parent) {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => number(map.get(field.key), anomalies),
        Some(_) => {
            anomalies.invalid_number = true;
            None
        }
    }
}

fn optional_u64(value: Option<&Value>, partial: &mut bool) -> Option<u64> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Number(value)) => value.as_u64().or_else(|| {
            *partial = true;
            None
        }),
        Some(_) => {
            *partial = true;
            None
        }
    }
}

pub(crate) fn map_status(status: &str) -> ProviderResponseState {
    match status {
        "queued" => ProviderResponseState::Queued,
        "in_progress" => ProviderResponseState::InProgress,
        "completed" => ProviderResponseState::Completed,
        "incomplete" => ProviderResponseState::Incomplete,
        "failed" | "error" => ProviderResponseState::Failed,
        "cancelled" | "canceled" => ProviderResponseState::Cancelled,
        _ => ProviderResponseState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObservationInput, ObservationLimitValues, ObservationLimits};

    fn parse(body: &[u8]) -> ResponseObservation {
        parse_response(ObservationInput::new(body, ObservationLimits::default()))
    }

    fn raw_bytes(observation: &ResponseObservation) -> Option<&[u8]> {
        observation
            .raw_usage
            .as_ref()
            .map(RawProviderUsage::as_bytes)
    }

    #[test]
    fn preserves_exact_usage_and_normalizes_cached_reasoning() {
        let body = br#"{"id":"resp_anon","model":"gpt-5.6-sol","status":"completed","usage": { "input_tokens": 10, "input_tokens_details": {"cached_tokens": 3}, "output_tokens": 8, "output_tokens_details":{"reasoning_tokens":5}, "total_tokens":18},"unknown":"ignored"}"#;
        let observation = parse(body);
        assert_eq!(raw_bytes(&observation), Some(&b"{ \"input_tokens\": 10, \"input_tokens_details\": {\"cached_tokens\": 3}, \"output_tokens\": 8, \"output_tokens_details\":{\"reasoning_tokens\":5}, \"total_tokens\":18}"[..]));
        let usage = observation.normalized_usage.as_ref();
        assert_eq!(usage.and_then(|value| value.input_uncached), Some(7));
        assert_eq!(usage.and_then(|value| value.output_reasoning), Some(5));
        assert_eq!(usage.map(|value| value.status), Some(UsageStatus::Final));
        assert_eq!(observation.usage_status, UsageStatus::Final);
    }

    #[test]
    fn invalid_relations_are_partial_without_zero_defaults() {
        let body = br#"{"status":"completed","usage":{"input_tokens":2,"input_tokens_details":{"cached_tokens":9},"output_tokens":1,"output_tokens_details":{"reasoning_tokens":4},"total_tokens":99}}"#;
        let observation = parse(body);
        let usage = observation.normalized_usage.as_ref();
        assert_eq!(usage.and_then(|value| value.input_uncached), None);
        assert_eq!(usage.map(|value| value.status), Some(UsageStatus::Partial));
        assert!(usage.is_some_and(|value| value.anomalies.any()));
    }

    #[test]
    fn only_the_top_level_usage_member_is_retained_and_normalized() {
        let body = br#"{"id":"resp_shadow","status":"completed","tools":[{"type":"function","parameters":{"properties":{"usage":{"description":"CANARY"}}}}],"note":"usage","usage":{"input_tokens":12,"output_tokens":8,"total_tokens":20}}"#;
        let observation = parse(body);
        assert_eq!(
            raw_bytes(&observation),
            Some(&br#"{"input_tokens":12,"output_tokens":8,"total_tokens":20}"#[..])
        );
        let debug = format!("{:?}", observation.normalized_usage);
        assert!(!debug.contains("CANARY"));
        assert_eq!(observation.usage_status, UsageStatus::Final);
        assert_eq!(
            observation
                .normalized_usage
                .as_ref()
                .and_then(|usage| usage.input_total),
            Some(12)
        );
    }

    #[test]
    fn a_usage_object_without_recognised_components_is_unavailable() {
        let observation = parse(br#"{"status":"completed","usage":{}}"#);
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
        assert_eq!(
            observation.normalized_usage.map(|usage| usage.status),
            Some(UsageStatus::Unavailable)
        );
        let unknown = parse(br#"{"status":"completed","usage":{"foo":1}}"#);
        assert_eq!(unknown.usage_status, UsageStatus::Unavailable);
    }

    #[test]
    fn values_above_the_durable_range_are_overflow_without_a_component() {
        let observation = parse(
            br#"{"status":"completed","usage":{"input_tokens":9223372036854775808,"output_tokens":1}}"#,
        );
        let usage = observation
            .normalized_usage
            .unwrap_or_else(|| unavailable_with_anomalies(AnomalyFlags::default()));
        assert_eq!(usage.input_total, None);
        assert_eq!(usage.output_total, Some(1));
        assert!(usage.anomalies.overflow);
        assert!(!usage.anomalies.invalid_number);
        assert_eq!(observation.usage_status, UsageStatus::Partial);
    }

    #[test]
    fn a_fractional_value_is_invalid_but_not_an_overflow() {
        let observation =
            parse(br#"{"status":"completed","usage":{"input_tokens":10.5,"output_tokens":2}}"#);
        let usage = observation
            .normalized_usage
            .unwrap_or_else(|| unavailable_with_anomalies(AnomalyFlags::default()));
        assert_eq!(usage.input_total, None);
        assert!(usage.anomalies.invalid_number);
        assert!(!usage.anomalies.overflow);
    }

    #[test]
    fn usage_is_not_final_before_a_terminal_response() {
        let observation = parse(
            br#"{"status":"in_progress","usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}"#,
        );
        assert_eq!(observation.usage_status, UsageStatus::Partial);
        assert_eq!(
            observation.normalized_usage.map(|usage| usage.status),
            Some(UsageStatus::Final)
        );
    }

    #[test]
    fn one_oversized_string_keeps_the_rest_of_the_document() {
        let long = "x".repeat(20 * 1024);
        let body = format!(
            r#"{{"id":"resp_long","model":"gpt-5.6-sol","status":"completed","output":[{{"type":"output_text","text":"{long}"}}],"usage":{{"input_tokens":3,"output_tokens":1,"total_tokens":4}}}}"#
        );
        let observation = parse(body.as_bytes());
        assert_eq!(observation.status, ObservationStatus::Complete);
        assert_eq!(observation.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
        assert_eq!(observation.usage_status, UsageStatus::Final);
    }

    #[test]
    fn an_oversized_retained_string_is_dropped_and_marks_partial() {
        let long = "r".repeat(20 * 1024);
        let body = format!(r#"{{"id":"{long}","model":"gpt-5.6-sol","status":"completed"}}"#);
        let observation = parse(body.as_bytes());
        assert_eq!(observation.provider_response_id, None);
        assert_eq!(observation.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(observation.status, ObservationStatus::Partial);
    }

    #[test]
    fn an_escaped_nul_never_reaches_a_retained_field() {
        let observation =
            parse(br#"{"id":"resp_\u0000hidden","model":"gpt-5.6-sol","status":"completed"}"#);
        assert_eq!(observation.provider_response_id, None);
        assert_eq!(observation.status, ObservationStatus::Partial);
        assert_eq!(observation.model.as_deref(), Some("gpt-5.6-sol"));
    }

    #[test]
    fn a_compaction_output_item_is_detected_without_retaining_its_content() {
        let observation = parse(
            br#"{"id":"resp_compact","status":"completed","output":[{"type":"compaction","encrypted_content":"PRIVATE"}],"usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}"#,
        );

        assert!(observation.compaction_output_seen);
        let debug = format!("{observation:?}");
        assert!(!debug.contains("PRIVATE"));
    }

    #[test]
    fn duplicate_keys_are_malformed() {
        let observation = parse(
            br#"{"status":"completed","usage":{"input_tokens":1},"usage":{"input_tokens":2}}"#,
        );
        assert_eq!(observation.status, ObservationStatus::Malformed);
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
        assert!(observation.raw_usage.is_none());
    }

    #[test]
    fn an_oversized_usage_object_reports_a_resource_limit() {
        let limits = ObservationLimits::new(ObservationLimitValues {
            max_semantic_bytes: 8 * 1024,
            max_usage_bytes: 8,
            max_sse_event_bytes: 1024,
            max_sse_events: 8,
            max_json_depth: 16,
            max_json_items: 128,
            max_string_bytes: 64,
        })
        .unwrap_or_default();
        let observation = parse_response(ObservationInput::new(
            br#"{"status":"completed","usage":{"input_tokens":4,"output_tokens":2}}"#,
            limits,
        ));
        assert_eq!(observation.status, ObservationStatus::ResourceLimit);
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
        assert!(observation.raw_usage.is_none());
    }
}
