//! Responses v1 extraction, visibility linkage, and privacy contract tests.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::needless_lifetimes,
    clippy::redundant_closure_for_method_calls,
    reason = "these assertions intentionally fail loudly when a fixed extractor fixture violates its contract"
)]

use std::convert::TryFrom;

use tracepress_context::{
    ContextAnalysisLimitValues, ContextAnalysisLimits, ContextAnalysisReason,
    ContextAnalysisStatus, ContextBlockKind, ContextDigest, ContextOrigin, ContextRole,
    LogicalContextStatus, analyze_responses, analyze_with_lookup,
};

fn limits() -> ContextAnalysisLimits {
    ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
        max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        max_analysis_work_units: 100_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })
    .expect("test limits are valid")
}

fn raw<'request>(request: &'request [u8], start: u64, end: u64) -> &'request [u8] {
    let start = usize::try_from(start).expect("start fits usize");
    let end = usize::try_from(end).expect("end fits usize");
    &request[start..end]
}

#[test]
fn responses_blocks_keep_exact_original_spans_and_measure_safe_content() {
    let request = br#"{
      "model": "gpt-4o",
      "instructions": "Keep \\u0063ompact",
      "input": [
        {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]},
        {"type":"function_call","call_id":"call_1","name":"lookup","arguments":"{\"q\":\"x\"}"},
        {"type":"function_call_output","call_id":"call_1","output":"result"}
      ]
    }"#;

    let analysis = analyze_responses(request, limits());

    assert_eq!(analysis.status, ContextAnalysisStatus::Complete);
    assert_eq!(analysis.reason, None);
    assert_eq!(
        analysis.request_content_hash,
        ContextDigest::from_bytes(request)
    );
    assert_eq!(analysis.blocks.len(), 5);
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::ExplicitOnly
    );
    assert!(analysis.visibility.explicit_request_complete);

    let kinds: Vec<_> = analysis.blocks.iter().map(|block| block.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ContextBlockKind::Instructions,
            ContextBlockKind::Message,
            ContextBlockKind::Text,
            ContextBlockKind::ToolCall,
            ContextBlockKind::ToolResult,
        ]
    );

    let text = &analysis.blocks[2];
    assert_eq!(
        raw(
            request,
            text.locator.raw_value_start,
            text.locator.raw_value_end
        ),
        br#""hello""#
    );
    assert_eq!(text.raw_bytes, br#""hello""#.len() as u64);
    assert!(text.semantic_fingerprint.is_some());
    assert_eq!(text.role, Some(ContextRole::User));
    assert_eq!(text.origin, ContextOrigin::HumanAuthored);
    assert_eq!(text.parent_ordinal, Some(1));

    let tool_call = &analysis.blocks[3];
    assert_eq!(raw(request, tool_call.locator.raw_value_start, tool_call.locator.raw_value_end), br#"{"type":"function_call","call_id":"call_1","name":"lookup","arguments":"{\"q\":\"x\"}"}"#);
    assert_eq!(tool_call.role, Some(ContextRole::Tool));
    assert_eq!(tool_call.origin, ContextOrigin::AgentGenerated);
    assert_eq!(
        tool_call.tool_call_id.as_ref().map(|value| value.as_str()),
        Some("call_1")
    );
    assert_eq!(
        tool_call.tool_name.as_ref().map(|value| value.as_str()),
        Some("lookup")
    );
    assert!(tool_call.semantic_fingerprint.is_none());

    let tool_result = &analysis.blocks[4];
    assert_eq!(
        raw(
            request,
            tool_result.locator.raw_value_start,
            tool_result.locator.raw_value_end
        ),
        br#"{"type":"function_call_output","call_id":"call_1","output":"result"}"#
    );
    assert_eq!(tool_result.origin, ContextOrigin::ToolGenerated);
    assert!(tool_result.semantic_fingerprint.is_some());

    let debug = format!("{analysis:?}");
    assert!(!debug.contains("hello"));
    assert!(!debug.contains("call_1"));
    assert!(!debug.contains("lookup"));
}

#[test]
fn previous_response_linkage_is_presence_only_and_uses_the_caller_lookup() {
    let request = br#"{"model":"gpt-4o","previous_response_id":"resp-1","input":"next"}"#;
    let lookup = |response_id: &str| response_id == "resp-1";

    let analysis = analyze_with_lookup(request, limits(), &lookup);

    assert_eq!(analysis.status, ContextAnalysisStatus::Complete);
    assert_eq!(
        analysis.visibility.logical_context_status(),
        LogicalContextStatus::ProviderManagedPartial
    );
    assert!(analysis.visibility.uses_previous_response);
    assert_eq!(analysis.visibility_facts.provider_state_reference_count, 1);
    assert!(analysis.visibility_facts.reference_resolved_locally);
    let reference = analysis
        .visibility_facts
        .provider_state_reference
        .expect("previous response reference");
    assert!(reference.present);
    assert_eq!(reference.observed_response_id_match, Some(true));
    assert!(reference.reference_hash.is_some());
    let debug = format!("{analysis:?}");
    assert!(!debug.contains("resp-1"));
}

#[test]
fn unknown_items_keep_their_span_without_becoming_opaque() {
    let request = br#"{"input":[{"type":"future_context_item","payload":{"v":1}}]}"#;

    let analysis = analyze_responses(request, limits());

    assert_eq!(analysis.status, ContextAnalysisStatus::Partial);
    assert_eq!(
        analysis.reason,
        Some(ContextAnalysisReason::UnknownContextItem)
    );
    assert_eq!(analysis.blocks.len(), 1);
    let block = &analysis.blocks[0];
    assert_eq!(block.kind, ContextBlockKind::Unknown);
    assert_eq!(
        raw(
            request,
            block.locator.raw_value_start,
            block.locator.raw_value_end
        ),
        br#"{"type":"future_context_item","payload":{"v":1}}"#
    );
    assert!(!analysis.visibility.contains_opaque_items);
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::Unknown
    );
}

#[test]
fn malformed_input_preserves_the_request_hash_and_does_not_panic() {
    let request = [b'{', b'\xff'];

    let analysis = analyze_responses(&request, limits());

    assert_eq!(analysis.status, ContextAnalysisStatus::Malformed);
    assert_eq!(analysis.reason, Some(ContextAnalysisReason::Malformed));
    assert!(analysis.blocks.is_empty());
    assert_eq!(
        analysis.request_content_hash,
        ContextDigest::from_bytes(&request)
    );
}
