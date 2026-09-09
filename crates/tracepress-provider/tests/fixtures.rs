//! Fixture-driven provider observation contract tests.
#![allow(
    clippy::match_same_arms,
    clippy::panic,
    reason = "Fixture dispatch preserves each named source path and fails loudly for invalid test names."
)]

use serde_json::{Map, Value};
use tracepress_provider::{
    MAX_RETAINED_USAGE_BYTES, NormalizedUsage, ObservationInput, ObservationLimitValues,
    ObservationLimits, ObservationStatus, OpenAiResponsesV1Observer, ProviderObserver,
    ProviderResponseState, RawProviderUsage, ResponseObservation, StreamingObserver, UsageStatus,
    normalize_usage, parse_request,
};

/// Every fixture that carries a non-streamed response body.
const NON_STREAM_FIXTURES: [&str; 10] = [
    "completed_basic",
    "incomplete",
    "failed",
    "missing_usage",
    "cached_reasoning",
    "tool_calls",
    "parallel_tool_calls",
    "unknown_json_field",
    "echoed_usage",
    "large_bounded",
];

/// Every fixture that carries an SSE response body driven to a clean end of stream.
const STREAM_FIXTURES: [&str; 3] = ["completed_stream", "malformed_sse", "unknown_event"];

/// The complete key set every golden file must declare, so a dropped key fails the test.
const GOLDEN_KEYS: [&str; 13] = [
    "status",
    "state",
    "usage_status",
    "provider_response_id",
    "model",
    "incomplete_reason",
    "error_code",
    "input_total",
    "input_cached",
    "input_uncached",
    "output_total",
    "output_reasoning",
    "total",
];

fn limits() -> ObservationLimits {
    ObservationLimits::new(ObservationLimitValues {
        max_semantic_bytes: 8 * 1024 * 1024,
        max_usage_bytes: MAX_RETAINED_USAGE_BYTES,
        max_sse_event_bytes: 1024 * 1024,
        max_sse_events: 100_000,
        max_json_depth: 64,
        max_json_items: 100_000,
        max_string_bytes: 16 * 1024,
    })
    .unwrap_or_default()
}

/// One golden expectation, fully declared: every field is asserted, absence included.
struct Golden {
    name: String,
    status: ObservationStatus,
    state: ProviderResponseState,
    usage_status: UsageStatus,
    provider_response_id: Option<String>,
    model: Option<String>,
    incomplete_reason: Option<String>,
    error_code: Option<String>,
    input_total: Option<u64>,
    input_cached: Option<u64>,
    input_uncached: Option<u64>,
    output_total: Option<u64>,
    output_reasoning: Option<u64>,
    total: Option<u64>,
}

fn expected_fixture(name: &str) -> Golden {
    let bytes: &'static [u8] = match name {
        "completed_basic" => {
            include_bytes!("../fixtures/openai/responses/v1/completed_basic/expected.json")
        }
        "completed_stream" => {
            include_bytes!("../fixtures/openai/responses/v1/completed_stream/expected.json")
        }
        "incomplete" => include_bytes!("../fixtures/openai/responses/v1/incomplete/expected.json"),
        "failed" => include_bytes!("../fixtures/openai/responses/v1/failed/expected.json"),
        "missing_usage" => {
            include_bytes!("../fixtures/openai/responses/v1/missing_usage/expected.json")
        }
        "cached_reasoning" => {
            include_bytes!("../fixtures/openai/responses/v1/cached_reasoning/expected.json")
        }
        "cancellation" => {
            include_bytes!("../fixtures/openai/responses/v1/cancellation/expected.json")
        }
        "tool_calls" => include_bytes!("../fixtures/openai/responses/v1/tool_calls/expected.json"),
        "parallel_tool_calls" => {
            include_bytes!("../fixtures/openai/responses/v1/parallel_tool_calls/expected.json")
        }
        "unknown_json_field" => {
            include_bytes!("../fixtures/openai/responses/v1/unknown_json_field/expected.json")
        }
        "echoed_usage" => {
            include_bytes!("../fixtures/openai/responses/v1/echoed_usage/expected.json")
        }
        "malformed_sse" => {
            include_bytes!("../fixtures/openai/responses/v1/malformed_sse/expected.json")
        }
        "unknown_event" => {
            include_bytes!("../fixtures/openai/responses/v1/unknown_event/expected.json")
        }
        "large_bounded" => {
            include_bytes!("../fixtures/openai/responses/v1/large_bounded/expected.json")
        }
        _ => panic!("unknown expected fixture: {name}"),
    };
    parse_golden(name, bytes)
}

fn parse_golden(name: &str, bytes: &[u8]) -> Golden {
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(bytes) else {
        panic!("golden {name} must be a JSON object");
    };
    for key in map.keys() {
        assert!(
            GOLDEN_KEYS.contains(&key.as_str()),
            "golden {name} declares unknown key {key}"
        );
    }
    Golden {
        name: name.to_owned(),
        status: observation_status(&map, name),
        state: response_state(&map, name),
        usage_status: usage_status(&map, name),
        provider_response_id: text(&map, "provider_response_id", name),
        model: text(&map, "model", name),
        incomplete_reason: text(&map, "incomplete_reason", name),
        error_code: text(&map, "error_code", name),
        input_total: count(&map, "input_total", name),
        input_cached: count(&map, "input_cached", name),
        input_uncached: count(&map, "input_uncached", name),
        output_total: count(&map, "output_total", name),
        output_reasoning: count(&map, "output_reasoning", name),
        total: count(&map, "total", name),
    }
}

fn required<'a>(map: &'a Map<String, Value>, key: &str, name: &str) -> &'a Value {
    let Some(value) = map.get(key) else {
        panic!("golden {name} must declare {key}");
    };
    value
}

fn text(map: &Map<String, Value>, key: &str, name: &str) -> Option<String> {
    match required(map, key, name) {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        other => panic!("golden {name} key {key} must be a string or null, found {other}"),
    }
}

fn count(map: &Map<String, Value>, key: &str, name: &str) -> Option<u64> {
    match required(map, key, name) {
        Value::Null => None,
        Value::Number(value) => {
            let Some(value) = value.as_u64() else {
                panic!("golden {name} key {key} must be a non-negative integer");
            };
            Some(value)
        }
        other => panic!("golden {name} key {key} must be a number or null, found {other}"),
    }
}

fn observation_status(map: &Map<String, Value>, name: &str) -> ObservationStatus {
    match text(map, "status", name).as_deref() {
        Some("Complete") => ObservationStatus::Complete,
        Some("Partial") => ObservationStatus::Partial,
        Some("Malformed") => ObservationStatus::Malformed,
        Some("ResourceLimit") => ObservationStatus::ResourceLimit,
        Some("Cancelled") => ObservationStatus::Cancelled,
        other => panic!("golden {name} declares unknown observation status {other:?}"),
    }
}

fn response_state(map: &Map<String, Value>, name: &str) -> ProviderResponseState {
    match text(map, "state", name).as_deref() {
        Some("Queued") => ProviderResponseState::Queued,
        Some("InProgress") => ProviderResponseState::InProgress,
        Some("Completed") => ProviderResponseState::Completed,
        Some("Incomplete") => ProviderResponseState::Incomplete,
        Some("Failed") => ProviderResponseState::Failed,
        Some("Cancelled") => ProviderResponseState::Cancelled,
        Some("Disconnected") => ProviderResponseState::Disconnected,
        Some("Unknown") => ProviderResponseState::Unknown,
        other => panic!("golden {name} declares unknown response state {other:?}"),
    }
}

fn usage_status(map: &Map<String, Value>, name: &str) -> UsageStatus {
    match text(map, "usage_status", name).as_deref() {
        Some("Final") => UsageStatus::Final,
        Some("Partial") => UsageStatus::Partial,
        Some("Unavailable") => UsageStatus::Unavailable,
        other => panic!("golden {name} declares unknown usage status {other:?}"),
    }
}

fn response_fixture(name: &str) -> (&'static [u8], Golden) {
    let response: &'static [u8] = match name {
        "completed_basic" => {
            include_bytes!("../fixtures/openai/responses/v1/completed_basic/response.json")
        }
        "incomplete" => include_bytes!("../fixtures/openai/responses/v1/incomplete/response.json"),
        "failed" => include_bytes!("../fixtures/openai/responses/v1/failed/response.json"),
        "missing_usage" => {
            include_bytes!("../fixtures/openai/responses/v1/missing_usage/response.json")
        }
        "cached_reasoning" => {
            include_bytes!("../fixtures/openai/responses/v1/cached_reasoning/response.json")
        }
        "tool_calls" => include_bytes!("../fixtures/openai/responses/v1/tool_calls/response.json"),
        "parallel_tool_calls" => {
            include_bytes!("../fixtures/openai/responses/v1/parallel_tool_calls/response.json")
        }
        "unknown_json_field" => {
            include_bytes!("../fixtures/openai/responses/v1/unknown_json_field/response.json")
        }
        "echoed_usage" => {
            include_bytes!("../fixtures/openai/responses/v1/echoed_usage/response.json")
        }
        "large_bounded" => {
            include_bytes!("../fixtures/openai/responses/v1/large_bounded/response.json")
        }
        _ => panic!("unknown response fixture: {name}"),
    };
    (response, expected_fixture(name))
}

fn request_fixture(name: &str) -> &'static [u8] {
    let request: &'static [u8] = match name {
        "completed_basic" => {
            include_bytes!("../fixtures/openai/responses/v1/completed_basic/request.json")
        }
        "completed_stream" => {
            include_bytes!("../fixtures/openai/responses/v1/completed_stream/request.json")
        }
        "incomplete" => include_bytes!("../fixtures/openai/responses/v1/incomplete/request.json"),
        "failed" => include_bytes!("../fixtures/openai/responses/v1/failed/request.json"),
        "missing_usage" => {
            include_bytes!("../fixtures/openai/responses/v1/missing_usage/request.json")
        }
        "cached_reasoning" => {
            include_bytes!("../fixtures/openai/responses/v1/cached_reasoning/request.json")
        }
        "cancellation" => {
            include_bytes!("../fixtures/openai/responses/v1/cancellation/request.json")
        }
        "tool_calls" => include_bytes!("../fixtures/openai/responses/v1/tool_calls/request.json"),
        "parallel_tool_calls" => {
            include_bytes!("../fixtures/openai/responses/v1/parallel_tool_calls/request.json")
        }
        "unknown_json_field" => {
            include_bytes!("../fixtures/openai/responses/v1/unknown_json_field/request.json")
        }
        "echoed_usage" => {
            include_bytes!("../fixtures/openai/responses/v1/echoed_usage/request.json")
        }
        "malformed_sse" => {
            include_bytes!("../fixtures/openai/responses/v1/malformed_sse/request.json")
        }
        "unknown_event" => {
            include_bytes!("../fixtures/openai/responses/v1/unknown_event/request.json")
        }
        "large_bounded" => {
            include_bytes!("../fixtures/openai/responses/v1/large_bounded/request.json")
        }
        _ => panic!("unknown request fixture: {name}"),
    };
    request
}

fn assert_request_is_safe_and_loaded(name: &str) {
    let observation = parse_request(ObservationInput::new(request_fixture(name), limits()));
    assert_eq!(
        observation.provider,
        tracepress_provider::ProviderKind::OpenAi
    );
    assert_eq!(
        observation.protocol,
        tracepress_provider::ProviderProtocol::OpenAiResponsesV1
    );
    assert_eq!(observation.status, ObservationStatus::Complete, "{name}");
    assert_eq!(observation.model.as_deref(), Some("gpt-5.6-sol"), "{name}");
    assert_eq!(observation.input_item_count, Some(1), "{name}");
    assert_eq!(observation.text_input_blocks, Some(1), "{name}");
    assert_eq!(observation.image_input_blocks, Some(0), "{name}");
    assert_eq!(observation.file_input_blocks, Some(0), "{name}");
}

fn assert_expected(observation: &ResponseObservation, expected: &Golden) {
    let name = expected.name.as_str();
    assert_eq!(observation.status, expected.status, "status of {name}");
    assert_eq!(
        observation.response_state, expected.state,
        "state of {name}"
    );
    assert_eq!(
        observation.usage_status, expected.usage_status,
        "usage status of {name}"
    );
    assert_eq!(
        observation.provider_response_id, expected.provider_response_id,
        "provider response id of {name}"
    );
    assert_eq!(observation.model, expected.model, "model of {name}");
    assert_eq!(
        observation.incomplete_reason, expected.incomplete_reason,
        "incomplete reason of {name}"
    );
    assert_eq!(
        observation.error_code, expected.error_code,
        "error code of {name}"
    );
    let usage = observation.normalized_usage.as_ref();
    for (key, actual, expected_value) in [
        (
            "input_total",
            usage.and_then(|value| value.input_total),
            expected.input_total,
        ),
        (
            "input_cached",
            usage.and_then(|value| value.input_cached),
            expected.input_cached,
        ),
        (
            "input_uncached",
            usage.and_then(|value| value.input_uncached),
            expected.input_uncached,
        ),
        (
            "output_total",
            usage.and_then(|value| value.output_total),
            expected.output_total,
        ),
        (
            "output_reasoning",
            usage.and_then(|value| value.output_reasoning),
            expected.output_reasoning,
        ),
        ("total", usage.and_then(|value| value.total), expected.total),
    ] {
        assert_eq!(actual, expected_value, "usage field {key} of {name}");
    }
}

fn semantic(
    observation: &ResponseObservation,
) -> (
    ObservationStatus,
    ProviderResponseState,
    UsageStatus,
    Option<&str>,
    Option<&str>,
    Option<&RawProviderUsage>,
) {
    (
        observation.status,
        observation.response_state,
        observation.usage_status,
        observation.provider_response_id.as_deref(),
        observation.model.as_deref(),
        observation.raw_usage.as_ref(),
    )
}

fn stream_fixture(name: &str) -> &'static [u8] {
    let response: &'static [u8] = match name {
        "completed_stream" => {
            include_bytes!("../fixtures/openai/responses/v1/completed_stream/response.sse")
        }
        "cancellation" => {
            include_bytes!("../fixtures/openai/responses/v1/cancellation/response.sse")
        }
        "malformed_sse" => {
            include_bytes!("../fixtures/openai/responses/v1/malformed_sse/response.sse")
        }
        "unknown_event" => {
            include_bytes!("../fixtures/openai/responses/v1/unknown_event/response.sse")
        }
        _ => panic!("unknown stream fixture: {name}"),
    };
    response
}

fn stream_chunks(bytes: &[u8], width: usize) -> Vec<&[u8]> {
    bytes.chunks(width).collect()
}

fn stream_with_boundaries(bytes: &[u8], boundaries: &[usize]) -> ResponseObservation {
    let mut observer = StreamingObserver::new(limits());
    let mut start = 0;
    for &width in boundaries {
        if start == bytes.len() {
            break;
        }
        let end = start.saturating_add(width).min(bytes.len());
        let Some(chunk) = bytes.get(start..end) else {
            panic!("fragment boundary {start}..{end} is outside the fixture");
        };
        assert!(observer.push(chunk).is_ok());
        start = end;
    }
    if start < bytes.len() {
        let Some(chunk) = bytes.get(start..) else {
            panic!("fragment tail {start} is outside the fixture");
        };
        assert!(observer.push(chunk).is_ok());
    }
    observer.finish()
}

fn observe_response(bytes: &[u8], limits: ObservationLimits, name: &str) -> ResponseObservation {
    match OpenAiResponsesV1Observer::new().observe_response(ObservationInput::new(bytes, limits)) {
        Ok(observation) => observation,
        Err(error) => panic!("observing {name} must not fail: {error}"),
    }
}

#[test]
fn every_non_stream_fixture_is_loaded_and_matches_golden_semantics() {
    for name in NON_STREAM_FIXTURES {
        assert_request_is_safe_and_loaded(name);
        let (bytes, expected) = response_fixture(name);
        let observation = observe_response(bytes, limits(), name);
        assert_expected(&observation, &expected);
    }
}

#[test]
fn an_echoed_client_usage_key_is_never_retained_as_provider_usage() {
    let (bytes, expected) = response_fixture("echoed_usage");
    let observation = observe_response(bytes, limits(), "echoed_usage");
    assert_expected(&observation, &expected);
    let retained = observation
        .raw_usage
        .as_ref()
        .map(|usage| usage.as_bytes().to_vec());
    assert_eq!(
        retained,
        Some(br#"{"input_tokens":9,"output_tokens":6,"total_tokens":15}"#.to_vec()),
        "only the provider's own top-level usage object may be retained"
    );
    let text = String::from_utf8(
        observation
            .raw_usage
            .as_ref()
            .map(|usage| usage.as_bytes().to_vec())
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    assert!(
        !text.contains("redacted"),
        "retained usage must not carry echoed client text"
    );
}

#[test]
fn stream_fixtures_are_invariant_for_fixed_and_seeded_random_fragmentation() {
    for name in STREAM_FIXTURES {
        assert_request_is_safe_and_loaded(name);
        let bytes = stream_fixture(name);
        let expected = expected_fixture(name);
        let one = OpenAiResponsesV1Observer::new().observe_stream(limits(), &[bytes]);
        assert_expected(&one, &expected);
        for width in [1, 2, 7, 64] {
            let chunks = stream_chunks(bytes, width);
            let fragmented = OpenAiResponsesV1Observer::new().observe_stream(limits(), &chunks);
            assert_eq!(
                semantic(&fragmented),
                semantic(&one),
                "fragment width {width} for {name}"
            );
            assert_eq!(fragmented.byte_count, Some(bytes.len() as u64));
        }
        let mut seed = 0x5eed_u64;
        for _ in 0..32 {
            let mut boundaries = Vec::new();
            for _ in 0..16 {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let width = usize::try_from(seed % 64)
                    .unwrap_or_default()
                    .saturating_add(1);
                boundaries.push(width);
            }
            let fragmented = stream_with_boundaries(bytes, &boundaries);
            assert_eq!(
                semantic(&fragmented),
                semantic(&one),
                "seeded boundaries for {name}"
            );
        }
    }
}

#[test]
fn cancellation_fixture_stays_cancelled_without_provider_completion() {
    assert_request_is_safe_and_loaded("cancellation");
    let bytes = stream_fixture("cancellation");
    let mut observer = StreamingObserver::new(limits());
    for chunk in bytes.chunks(7) {
        assert!(observer.push(chunk).is_ok());
    }
    let expected = expected_fixture("cancellation");
    let observation = observer.cancel();
    assert_expected(&observation, &expected);
}

#[test]
fn a_completed_stream_keeps_its_evidence_when_the_client_stops_reading() {
    let bytes = stream_fixture("completed_stream");
    let expected = expected_fixture("completed_stream");
    let mut observer = StreamingObserver::new(limits());
    for chunk in bytes.chunks(5) {
        assert!(observer.push(chunk).is_ok());
    }
    let disconnected = observer.disconnect();
    assert_expected(&disconnected, &expected);
    assert!(
        disconnected.raw_usage.is_some(),
        "final usage bytes must survive a disconnect after completion"
    );
    let cancelled = observer.cancel();
    assert_eq!(semantic(&cancelled), semantic(&disconnected));
}

#[test]
fn a_streamed_run_reports_measured_and_ordered_timings() {
    let bytes = stream_fixture("completed_stream");
    let mut observer = StreamingObserver::new(limits());
    let mut first = true;
    for chunk in bytes.chunks(24) {
        if !first {
            std::thread::sleep(std::time::Duration::from_micros(1200));
        }
        first = false;
        assert!(observer.push(chunk).is_ok());
    }
    let observation = observer.finish();
    let first_byte = observation.ttfb_us;
    let first_output = observation.ttft_us;
    let duration = observation.duration_us;
    assert!(first_byte.is_some(), "time to first byte must be measured");
    assert!(
        first_output > first_byte,
        "time to first token {first_output:?} must follow first byte {first_byte:?}"
    );
    assert!(
        duration > first_output,
        "duration {duration:?} must follow first token {first_output:?}"
    );
}

#[test]
fn large_fixture_is_bounded_and_rejects_a_too_small_limit() {
    let bytes = include_bytes!("../fixtures/openai/responses/v1/large_bounded/response.json");
    assert!(bytes.len() > 4096);
    let observation = observe_response(bytes, limits(), "large_bounded");
    assert_eq!(observation.response_state, ProviderResponseState::Completed);
    let constrained = ObservationLimits::new(ObservationLimitValues {
        max_semantic_bytes: 1024,
        max_usage_bytes: limits().max_usage_bytes,
        max_sse_event_bytes: limits().max_sse_event_bytes,
        max_sse_events: limits().max_sse_events,
        max_json_depth: limits().max_json_depth,
        max_json_items: limits().max_json_items,
        max_string_bytes: limits().max_string_bytes,
    })
    .unwrap_or_default();
    let bounded = observe_response(bytes, constrained, "large_bounded");
    assert_eq!(bounded.status, ObservationStatus::ResourceLimit);
}

#[test]
fn a_document_carrying_one_long_string_still_yields_its_allowlisted_metadata() {
    let long = "x".repeat(20 * 1024);
    let body = format!(
        r#"{{"id":"resp_long","model":"gpt-5.6-sol","status":"completed","output":[{{"type":"message","content":[{{"type":"output_text","text":"{long}"}}]}}],"usage":{{"input_tokens":11,"output_tokens":4,"total_tokens":15}}}}"#
    );
    let observation = observe_response(body.as_bytes(), limits(), "long_string");
    assert_eq!(observation.status, ObservationStatus::Complete);
    assert_eq!(
        observation.provider_response_id.as_deref(),
        Some("resp_long")
    );
    assert_eq!(observation.model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(observation.response_state, ProviderResponseState::Completed);
    assert_eq!(observation.usage_status, UsageStatus::Final);
    assert_eq!(
        observation
            .normalized_usage
            .as_ref()
            .and_then(|usage| usage.total),
        Some(15)
    );

    let request = format!(
        r#"{{"model":"gpt-5.6-sol","stream":true,"input":[{{"role":"user","content":[{{"type":"input_text","text":"{long}"}}]}}]}}"#
    );
    let observed = parse_request(ObservationInput::new(request.as_bytes(), limits()));
    assert_eq!(observed.status, ObservationStatus::Complete);
    assert_eq!(observed.model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(observed.stream, Some(true));
    assert_eq!(observed.text_input_blocks, Some(1));
}

#[test]
fn an_oversized_streamed_identifier_is_dropped_and_marks_the_observation_partial() {
    let long = "i".repeat(20 * 1024);
    let stream = format!(
        "event: response.completed\ndata: {{\"response\":{{\"id\":\"{long}\",\"model\":\"gpt-5.6-sol\",\"status\":\"completed\",\"usage\":{{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}}}}\n\n"
    );
    let observation =
        OpenAiResponsesV1Observer::new().observe_stream(limits(), &[stream.as_bytes()]);
    assert_eq!(observation.provider_response_id, None);
    assert_eq!(observation.model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(observation.status, ObservationStatus::Partial);
    assert_eq!(observation.response_state, ProviderResponseState::Completed);
    assert_eq!(observation.usage_status, UsageStatus::Final);
}

#[test]
fn usage_normalization_keeps_unknown_values_absent() {
    let usage = normalized(
        br#"{"input_tokens":4,"future_tokens":"ignored","output_tokens":2,"total_tokens":6}"#,
    );
    assert_eq!(usage.input_total, Some(4));
    assert_eq!(usage.output_total, Some(2));
    assert_eq!(usage.total, Some(6));
    assert_eq!(usage.input_cached, None);
    assert_eq!(usage.status, UsageStatus::Final);
}

#[test]
fn a_usage_object_with_no_recognised_component_is_unavailable() {
    assert_eq!(normalized(br"{}").status, UsageStatus::Unavailable);
    assert_eq!(
        normalized(br#"{"unknown_tokens":5}"#).status,
        UsageStatus::Unavailable
    );
}

#[test]
fn a_usage_value_above_the_durable_range_is_an_overflow_anomaly() {
    let usage = normalized(br#"{"input_tokens":9223372036854775808,"output_tokens":2}"#);
    assert_eq!(usage.input_total, None);
    assert_eq!(usage.output_total, Some(2));
    assert!(usage.anomalies.overflow);
    assert_eq!(usage.status, UsageStatus::Partial);
}

fn normalized(bytes: &[u8]) -> NormalizedUsage {
    match RawProviderUsage::new(bytes, limits().max_usage_bytes) {
        Ok(raw) => normalize_usage(&raw),
        Err(error) => panic!("usage fixture must be retainable: {error}"),
    }
}

#[test]
fn request_fixture_metadata_does_not_retain_prompt_or_tool_values() {
    let observation = parse_request(ObservationInput::new(
        request_fixture("tool_calls"),
        limits(),
    ));
    let debug = format!("{observation:?}");
    assert!(!debug.contains("lookup_weather"));
    assert!(!debug.contains("redacted"));
    assert_eq!(observation.tool_count, Some(1));
}
