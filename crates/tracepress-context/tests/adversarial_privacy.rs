//! Adversarial, privacy, and resource-bound contract coverage for context analysis.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "fixed adversarial fixtures should fail loudly when the contract changes"
)]

use std::{convert::TryFrom, fmt::Write as _};

use serde_json::to_string;
use tracepress_context::{
    ContextAnalysisLimitReason, ContextAnalysisLimitValues, ContextAnalysisLimits,
    ContextAnalysisReason, ContextAnalysisStatus, ContextBlockKind, ContextDigest, ContextOrigin,
    ContextVisibility, LogicalContextStatus, MeasurementApplicability, OpportunitySignal,
    analyze_responses, analyze_with_lookup,
};

fn limits() -> ContextAnalysisLimits {
    limits_with(
        ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
        ContextAnalysisLimits::MAX_BLOCKS,
        ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
    )
}

fn limits_with(
    max_analyzed_bytes: u64,
    max_blocks: u64,
    max_string_bytes_inspected: u64,
) -> ContextAnalysisLimits {
    ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes,
        max_blocks,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected,
        max_analysis_work_units: 1_000_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })
    .expect("adversarial test limits are valid")
}

fn raw(request: &[u8], start: u64, end: u64) -> &[u8] {
    let start = usize::try_from(start).expect("start fits usize");
    let end = usize::try_from(end).expect("end fits usize");
    &request[start..end]
}

#[test]
fn analysis_byte_budget_is_honest_without_changing_the_forwarded_bytes() {
    let canary = "CANARY_OVERSIZED_CONTEXT_9A7F";
    let request = format!(
        "{{\"model\":\"gpt-4o\",\"input\":\"{}{}\"}}",
        "prefix-",
        format!("{canary}-").repeat(20_000)
    );
    let original = request.as_bytes().to_vec();
    let analysis = analyze_responses(request.as_bytes(), limits_with(256, 8_192, 65_536));

    assert_eq!(analysis.status, ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        analysis.reason,
        Some(ContextAnalysisReason::ResourceLimit {
            limit: ContextAnalysisLimitReason::AnalyzedBytes,
        })
    );
    assert!(analysis.analyzed_bytes <= 256);
    assert!(analysis.skipped_bytes > 0);
    assert_eq!(
        analysis
            .analyzed_bytes
            .saturating_add(analysis.skipped_bytes),
        u64::try_from(original.len()).expect("request length fits u64")
    );
    assert_eq!(
        analysis.request_content_hash,
        ContextDigest::from_bytes(&original)
    );
    assert_eq!(request.as_bytes(), original.as_slice());
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::Unknown
    );

    let debug = format!("{analysis:?}");
    let wire = to_string(&analysis).expect("analysis serializes");
    assert!(!debug.contains(canary));
    assert!(!wire.contains(canary));
}

#[test]
fn five_thousand_explicit_blocks_remain_bounded_and_ordered() {
    // A repeated root key creates one explicit text block per occurrence while the span index
    // still admits every value under the fixed 8,192-node structural bound.
    let mut request = String::from(r#"{"model":"gpt-4o","#);
    for index in 0..5_000 {
        if index != 0 {
            request.push(',');
        }
        write!(request, r#""input":"block-{index}""#).expect("request fixture writes");
    }
    request.push('}');

    let analysis = analyze_responses(request.as_bytes(), limits());

    assert_eq!(analysis.status, ContextAnalysisStatus::Partial);
    assert_eq!(analysis.reason, Some(ContextAnalysisReason::DuplicateKey));
    assert_eq!(analysis.blocks.len(), 5_000);
    assert_eq!(analysis.explicit_block_count(), 5_000);
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::Unknown
    );
    assert!(
        analysis
            .blocks
            .iter()
            .enumerate()
            .all(|(index, block)| block.ordinal == u32::try_from(index).expect("ordinal fits u32"))
    );
    assert!(analysis.blocks.iter().all(|block| {
        block.locator.raw_value_end <= u64::try_from(request.len()).expect("length fits u64")
            && block.raw_bytes
                == block
                    .locator
                    .raw_value_end
                    .saturating_sub(block.locator.raw_value_start)
    }));
}

#[test]
fn block_limit_preserves_maximum_prefix_after_structural_owner_slots() {
    let mut request = String::from(r#"{"model":"gpt-4o","#);
    for index in 0..8_193 {
        if index != 0 {
            request.push(',');
        }
        write!(request, r#""input":"block-{index}""#).expect("request fixture writes");
    }
    request.push('}');

    let analysis = analyze_responses(request.as_bytes(), limits());

    assert_eq!(analysis.status, ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        analysis.reason,
        Some(ContextAnalysisReason::ResourceLimit {
            limit: ContextAnalysisLimitReason::Blocks,
        })
    );
    assert_eq!(analysis.blocks.len(), 8_192);
    assert_eq!(analysis.blocks[0].ordinal, 0);
    assert_eq!(analysis.blocks[8_191].ordinal, 8_191);
}

#[test]
fn duplicate_keys_preserve_occurrences_and_spans() {
    let request = br#"{"input":[{"type":"input_text","text":"first"}],"input":[{"type":"input_text","text":"second"}]}"#;
    let analysis = analyze_responses(request, limits());

    assert_eq!(analysis.status, ContextAnalysisStatus::Partial);
    assert_eq!(analysis.reason, Some(ContextAnalysisReason::DuplicateKey));
    assert!(analysis.duplicate_key_detected);
    assert_eq!(analysis.blocks.len(), 2);
    let spans: Vec<_> = analysis
        .blocks
        .iter()
        .map(|block| {
            raw(
                request,
                block.locator.raw_value_start,
                block.locator.raw_value_end,
            )
        })
        .collect();
    assert_eq!(spans, vec![&br#""first""#[..], &br#""second""#[..]]);
    assert_ne!(
        analysis.blocks[0].locator.raw_value_start,
        analysis.blocks[1].locator.raw_value_start
    );
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::Unknown
    );
}

#[test]
fn malformed_utf8_raw_nul_and_escaped_nul_are_distinguished() {
    let malformed_cases: &[&[u8]] = &[
        br#"{"input":"unterminated}"#,
        b"{\xff",
        b"{\"input\":\"raw\0nul\"}",
    ];
    for request in malformed_cases {
        let analysis = analyze_responses(request, limits());
        assert_eq!(analysis.status, ContextAnalysisStatus::Malformed);
        assert_eq!(analysis.reason, Some(ContextAnalysisReason::Malformed));
        assert!(analysis.blocks.is_empty());
        assert_eq!(
            analysis.request_content_hash,
            ContextDigest::from_bytes(request)
        );
    }

    let escaped_nul = br#"{"input":"escaped\u0000nul"}"#;
    let analysis = analyze_responses(escaped_nul, limits());
    assert_eq!(analysis.status, ContextAnalysisStatus::Complete);
    assert_eq!(analysis.reason, None);
    assert_eq!(analysis.blocks.len(), 1);
    assert!(analysis.blocks[0].semantic_fingerprint.is_some());
}

#[test]
fn huge_tool_payloads_and_schemas_only_emit_bounded_features_and_signals() {
    let result_padding = "result-line ".repeat(1_000);
    let schema_padding = "schema-description ".repeat(400);
    let result_canary = "CANARY_TOOL_RESULT_PAYLOAD_61C2";
    let schema_canary = "CANARY_TOOL_SCHEMA_DESCRIPTION_18E4";
    let request = format!(
        r#"{{"model":"gpt-4o","tools":[{{"type":"function","name":"safe_tool","description":"{schema_canary}{schema_padding}","parameters":{{"type":"object","properties":{{"field":{{"type":"string","description":"{schema_canary}"}}}}}}}}],"input":[{{"type":"function_call_output","call_id":"safe-call","output":{{"result":"{result_canary}","lines":"{result_padding}"}}}}]}}"#
    );
    let analysis = analyze_responses(request.as_bytes(), limits());

    assert_eq!(analysis.status, ContextAnalysisStatus::Complete);
    let result = analysis
        .blocks
        .iter()
        .find(|block| block.kind == ContextBlockKind::ToolResult)
        .expect("tool result block");
    let result_features = result.features.expect("bounded result features");
    assert!(result_features.raw_bytes >= OpportunitySignal::LARGE_BLOCK_BYTES);
    assert!(
        result
            .opportunity_signals
            .contains(OpportunitySignal::LargeToolResult)
    );

    let schema = analysis
        .blocks
        .iter()
        .find(|block| block.kind == ContextBlockKind::ToolDefinition)
        .expect("tool schema block");
    let schema_features = schema.features.expect("bounded schema features");
    assert!(schema_features.raw_bytes >= OpportunitySignal::LARGE_TOOL_SCHEMA_BYTES);
    assert!(
        schema
            .opportunity_signals
            .contains(OpportunitySignal::LargeToolSchema)
    );
    assert!(result.token_estimate.is_some());
    assert!(schema.token_estimate.is_some());

    let debug = format!("{analysis:?}");
    let wire = to_string(&analysis).expect("analysis serializes");
    assert!(!debug.contains(result_canary));
    assert!(!debug.contains(schema_canary));
    assert!(!wire.contains(result_canary));
    assert!(!wire.contains(schema_canary));
}

#[test]
fn string_inspection_bound_keeps_unknown_measurements_unknown() {
    let canary = "CANARY_OVERLONG_TEXT_4B9D";
    let value = format!("{}{}", canary, "x".repeat(2_000));
    let request = format!(r#"{{"input":"{value}"}}"#);
    let analysis = analyze_responses(request.as_bytes(), limits_with(4_096, 64, 32));
    assert_eq!(analysis.status, ContextAnalysisStatus::ResourceLimit);
    assert!(!analysis.visibility.explicit_request_complete);
    assert_eq!(
        analysis.reason,
        Some(ContextAnalysisReason::ResourceLimit {
            limit: ContextAnalysisLimitReason::StringBytesInspected,
        })
    );
    assert_eq!(analysis.blocks.len(), 1);
    let block = &analysis.blocks[0];
    assert_eq!(block.kind, ContextBlockKind::Text);
    assert_eq!(
        block.detection_applicability,
        MeasurementApplicability::Eligible
    );
    assert_eq!(
        block.token_estimation_applicability,
        MeasurementApplicability::Eligible
    );
    assert!(block.semantic_fingerprint.is_none());
    assert!(block.token_estimate.is_none());
    assert!(block.detection_result.is_none());
    assert!(block.features.is_none());
    assert!(block.opportunity_signals.is_empty());
    let debug = format!("{analysis:?}");
    let wire = to_string(&analysis).expect("analysis serializes");
    assert!(!debug.contains(canary));
    assert!(!wire.contains(canary));
}

#[test]
fn extraction_budget_keeps_completed_drafts_before_wide_measurement_exhaustion() {
    let wide = "x".repeat(20_000);
    let request =
        format!(r#"{{"input":"tiny","input":"{wide}","input":"{wide}","input":"{wide}"}}"#);
    let limits = ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
        max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        max_analysis_work_units: 40,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })
    .expect("bounded extraction test limits");

    let analysis = analyze_responses(request.as_bytes(), limits);
    assert_eq!(analysis.blocks.len(), 3);
    assert!(analysis.blocks[0].token_estimate.is_some());
    assert!(analysis.blocks[1].token_estimate.is_some());
    assert_eq!(
        analysis.blocks[2].detection_applicability,
        MeasurementApplicability::Eligible
    );
    assert_eq!(
        analysis.blocks[2].token_estimation_applicability,
        MeasurementApplicability::Eligible
    );
    assert!(analysis.blocks[2].semantic_fingerprint.is_none());
    assert!(analysis.blocks[2].token_estimate.is_none());
    assert!(analysis.blocks[2].detection_result.is_none());
    assert!(analysis.blocks[2].features.is_none());
    assert!(analysis.blocks[2].opportunity_signals.is_empty());
}

#[test]
fn provider_and_external_references_never_claim_explicit_only_or_fetch_content() {
    let canaries = [
        "CANARY_PREVIOUS_RESPONSE_01",
        "CANARY_CONVERSATION_STATE_02",
        "CANARY_ITEM_REFERENCE_03",
        "CANARY_PROMPT_REFERENCE_04",
        "CANARY_FILE_ID_05",
        "CANARY_IMAGE_URL_06",
        "CANARY_INPUT_FILE_07",
        "CANARY_INPUT_IMAGE_08",
    ];
    let request = format!(
        r#"{{"previous_response_id":"{}","conversation":"{}","item_reference":"{}","prompt":{{"id":"{}"}},"file_id":"{}","image_url":"https://example.invalid/{}","input":[{{"type":"message","role":"user","content":[{{"type":"item_reference","id":"{}"}},{{"type":"prompt","id":"{}"}},{{"type":"input_file","file_id":"{}"}},{{"type":"input_image","image_url":"https://example.invalid/{}"}}]}}]}}"#,
        canaries[0],
        canaries[1],
        canaries[2],
        canaries[3],
        canaries[4],
        canaries[5],
        canaries[6],
        canaries[3],
        canaries[6],
        canaries[7],
    );
    let analysis = analyze_with_lookup(request.as_bytes(), limits(), &|_: &str| false);

    assert_eq!(analysis.status, ContextAnalysisStatus::Complete);
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::MixedPartial
    );
    assert!(analysis.visibility.uses_previous_response);
    assert!(analysis.visibility.uses_conversation_state);
    assert!(analysis.visibility.uses_item_references);
    assert!(analysis.visibility.uses_prompt_reference);
    assert!(analysis.visibility.uses_external_files);
    assert!(analysis.visibility.uses_external_images);
    assert_ne!(
        analysis.logical_context_status(),
        LogicalContextStatus::ExplicitOnly
    );
    assert!(
        analysis
            .blocks
            .iter()
            .filter(|block| {
                matches!(
                    block.kind,
                    ContextBlockKind::FileReference
                        | ContextBlockKind::ImageReference
                        | ContextBlockKind::ItemReference
                        | ContextBlockKind::PromptReference
                        | ContextBlockKind::ProviderStateReference
                )
            })
            .all(|block| {
                block.detection_applicability == MeasurementApplicability::Ineligible
                    && block.token_estimation_applicability == MeasurementApplicability::Ineligible
                    && block.semantic_fingerprint.is_none()
                    && block.features.is_none()
                    && block.detection_result.is_none()
                    && block.token_estimate.is_none()
                    && block.origin != ContextOrigin::HumanAuthored
            })
    );
    let debug = format!("{analysis:?}");
    let wire = to_string(&analysis).expect("analysis serializes");
    for canary in canaries {
        assert!(!debug.contains(canary), "debug leaked {canary}");
        assert!(!wire.contains(canary), "wire leaked {canary}");
    }
}

#[test]
fn previous_response_linkage_reports_unresolved_without_promoting_visibility() {
    let request = br#"{"previous_response_id":"CANARY_UNRESOLVED_RESPONSE","input":"next"}"#;
    let analysis = analyze_with_lookup(request, limits(), &|_: &str| false);

    assert_eq!(analysis.status, ContextAnalysisStatus::Complete);
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::ProviderManagedPartial
    );
    assert_eq!(
        analysis
            .visibility_facts
            .provider_state_reference
            .expect("provider state fact")
            .observed_response_id_match,
        Some(false)
    );
    let debug = format!("{analysis:?}");
    assert!(!debug.contains("CANARY_UNRESOLVED_RESPONSE"));
}

#[test]
fn visibility_value_has_no_unexpected_raw_content_in_debug_or_wire() {
    let request = br#"{"instructions":"CANARY_INSTRUCTIONS_71","metadata":{"secret":"CANARY_METADATA_72"},"user":"CANARY_USER_73","prompt_cache_key":"CANARY_CACHE_74","safety_identifier":"CANARY_SAFETY_75","input":"CANARY_INPUT_76"}"#;
    let analysis = analyze_responses(request, limits());
    let expected = ContextVisibility {
        explicit_request_complete: true,
        uses_previous_response: false,
        uses_conversation_state: false,
        uses_item_references: false,
        uses_prompt_reference: false,
        uses_external_files: false,
        uses_external_images: false,
        contains_opaque_items: false,
    };
    assert_eq!(analysis.visibility, expected);
    assert_eq!(
        analysis.logical_context_status(),
        LogicalContextStatus::ExplicitOnly
    );
    let debug = format!("{analysis:?}");
    let wire = to_string(&analysis).expect("analysis serializes");
    for canary in [
        "CANARY_INSTRUCTIONS_71",
        "CANARY_METADATA_72",
        "CANARY_USER_73",
        "CANARY_CACHE_74",
        "CANARY_SAFETY_75",
        "CANARY_INPUT_76",
    ] {
        assert!(!debug.contains(canary));
        assert!(!wire.contains(canary));
    }
}
