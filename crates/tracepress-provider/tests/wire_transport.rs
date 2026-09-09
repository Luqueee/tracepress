//! Serialization of semantic observations for transport to a remote writer.

use tracepress_provider::{
    ObservationInput, ObservationLimits, ObservationStatus, ProviderKind, ProviderProtocol,
    ProviderResponseState, UsageStatus, parse_request, parse_response,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PROMPT_CANARY: &str = "prompt-canary-do-not-store";
const INSTRUCTION_CANARY: &str = "instruction-canary-do-not-store";
const OUTPUT_CANARY: &str = "output-canary-do-not-store";

fn limits() -> ObservationLimits {
    ObservationLimits::default()
}

fn request_body() -> String {
    format!(
        r#"{{"model":"gpt-test","stream":true,"instructions":"{INSTRUCTION_CANARY}","previous_response_id":"resp_prev","reasoning":{{"effort":"high"}},"input":[{{"role":"user","content":[{{"type":"input_text","text":"{PROMPT_CANARY}"}}]}}],"tools":[{{"type":"function","name":"{PROMPT_CANARY}"}}]}}"#
    )
}

fn response_body() -> String {
    format!(
        r#"{{"id":"resp_1","model":"gpt-test","status":"completed","output":[{{"type":"message","content":[{{"type":"output_text","text":"{OUTPUT_CANARY}"}}]}}],"usage":{{"input_tokens":12,"input_tokens_details":{{"cached_tokens":4}},"output_tokens":8,"output_tokens_details":{{"reasoning_tokens":5}},"total_tokens":20}}}}"#
    )
}

#[test]
fn request_observation_round_trips_without_carrying_content() -> TestResult {
    let body = request_body();
    let observation = parse_request(ObservationInput::new(body.as_bytes(), limits()));
    let wire = serde_json::to_string(&observation)?;

    for canary in [PROMPT_CANARY, INSTRUCTION_CANARY] {
        assert!(
            !wire.contains(canary),
            "serialized request observation leaked {canary}"
        );
    }
    assert!(
        !wire.contains("resp_prev"),
        "serialized previous response id"
    );

    let restored: tracepress_provider::RequestObservation = serde_json::from_str(&wire)?;
    assert_eq!(restored, observation);
    assert_eq!(restored.provider, ProviderKind::OpenAi);
    assert_eq!(restored.protocol, ProviderProtocol::OpenAiResponsesV1);
    assert_eq!(restored.status, ObservationStatus::Complete);
    assert_eq!(restored.request_bytes, Some(u64::try_from(body.len())?));
    assert_eq!(restored.reasoning_effort.as_deref(), Some("high"));
    assert!(restored.has_previous_response_id);
    Ok(())
}

#[test]
fn response_observation_round_trips_with_exact_usage_bytes() -> TestResult {
    let body = response_body();
    let observation = parse_response(ObservationInput::new(body.as_bytes(), limits()));
    let usage = observation
        .raw_usage
        .as_ref()
        .ok_or("response fixture must retain a usage object")?
        .as_bytes()
        .to_vec();
    let wire = serde_json::to_string(&observation)?;

    assert!(
        !wire.contains(OUTPUT_CANARY),
        "serialized response observation leaked {OUTPUT_CANARY}"
    );

    let restored: tracepress_provider::ResponseObservation = serde_json::from_str(&wire)?;
    assert_eq!(restored, observation);
    assert_eq!(restored.response_state, ProviderResponseState::Completed);
    assert_eq!(restored.usage_status, UsageStatus::Final);
    assert_eq!(
        restored
            .raw_usage
            .as_ref()
            .map(|raw| raw.as_bytes().to_vec()),
        Some(usage)
    );
    let normalized = restored
        .normalized_usage
        .as_ref()
        .ok_or("normalized usage must survive transport")?;
    assert_eq!(normalized.input_cached, Some(4));
    assert_eq!(normalized.output_reasoning, Some(5));
    assert_eq!(normalized.total, Some(20));
    Ok(())
}

#[test]
fn transported_usage_bytes_must_remain_a_json_object() {
    let rejected = serde_json::from_str::<tracepress_provider::RawProviderUsage>("[110,111]");
    assert!(
        rejected.is_err(),
        "non-object usage bytes must not deserialize"
    );
}
