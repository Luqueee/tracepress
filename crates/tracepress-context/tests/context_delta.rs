//! Observable contracts for bounded, multiset-aware context deltas.
#![allow(
    clippy::expect_used,
    reason = "invalid fixed test fixtures are fatal test-construction bugs"
)]

use tracepress_context::{
    ContextAnalysisLimitValues, ContextAnalysisLimits, ContextAnalysisStatus, ContextBlockKind,
    ContextBlockSummary, ContextDeltaRequest, ContextDeltaStatus, ContextDigest, ContextRole,
    JsonValueKind, RawSpan, SEMANTIC_FINGERPRINT_VERSION, SemanticFingerprint,
    SemanticFingerprintInput, compute_context_delta, semantic_fingerprint,
};
use tracepress_core::{ContextSnapshotId, UuidV7Generator};

fn limits(max_blocks: u64) -> ContextAnalysisLimits {
    ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
        max_blocks,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        max_analysis_work_units: 2_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })
    .expect("limits")
}

fn block(exact: &str, semantic_value: Option<&str>, tokens: Option<u64>) -> ContextBlockSummary {
    ContextBlockSummary {
        exact_fingerprint: ContextDigest::from_bytes(exact.as_bytes()),
        semantic_fingerprint: semantic_value.map(semantic),
        estimated_tokens: tokens,
    }
}

fn semantic(value: &str) -> SemanticFingerprint {
    let encoded = format!("\"{value}\"");
    semantic_fingerprint(SemanticFingerprintInput {
        request: encoded.as_bytes(),
        block_kind: ContextBlockKind::Text,
        role: ContextRole::User,
        value_span: RawSpan::new(0, u64::try_from(encoded.len()).expect("length")).expect("span"),
        value_kind: JsonValueKind::String,
        duplicate_key_in_subtree: false,
        fingerprint_version: SEMANTIC_FINGERPRINT_VERSION,
        limits: limits(ContextAnalysisLimits::MAX_BLOCKS),
    })
    .expect("semantic fingerprint")
}

#[derive(Clone, Copy)]
struct DeltaInput<'blocks> {
    previous: &'blocks [ContextBlockSummary],
    current: &'blocks [ContextBlockSummary],
    max_blocks: u64,
    previous_analysis_status: ContextAnalysisStatus,
    current_analysis_status: ContextAnalysisStatus,
}

fn delta(
    previous: &[ContextBlockSummary],
    current: &[ContextBlockSummary],
    max_blocks: u64,
) -> tracepress_context::ContextDelta {
    delta_with_status(DeltaInput {
        previous,
        current,
        max_blocks,
        previous_analysis_status: ContextAnalysisStatus::Complete,
        current_analysis_status: ContextAnalysisStatus::Complete,
    })
}

fn delta_with_status(input: DeltaInput<'_>) -> tracepress_context::ContextDelta {
    let generator = UuidV7Generator::new();
    compute_context_delta(ContextDeltaRequest {
        previous_snapshot_id: ContextSnapshotId::generate(&generator),
        current_snapshot_id: ContextSnapshotId::generate(&generator),
        previous: input.previous,
        current: input.current,
        previous_analysis_status: input.previous_analysis_status,
        current_analysis_status: input.current_analysis_status,
        limits: limits(input.max_blocks),
    })
}

#[test]
fn classifies_exact_changed_new_and_removed_blocks_deterministically() {
    let previous = [
        block("prefix", Some("prefix"), Some(2)),
        block("same", Some("same"), Some(3)),
        block("old-wire", Some("meaning"), Some(5)),
        block("removed", None, Some(7)),
    ];
    let current = [
        block("prefix", Some("prefix"), Some(2)),
        block("same", Some("same"), Some(3)),
        block("new-wire", Some("meaning"), Some(6)),
        block("new", None, Some(11)),
    ];

    let first = delta(&previous, &current, 32);
    let second = delta(&previous, &current, 32);
    assert_eq!(first.status, ContextDeltaStatus::Complete);
    assert_eq!(first.repeated_blocks, 2);
    assert_eq!(first.changed_blocks, 1);
    assert_eq!(first.new_blocks, 1);
    assert_eq!(first.removed_blocks, 1);
    assert_eq!(first.repeated_estimated_tokens, Some(5));
    assert_eq!(first.new_estimated_tokens, Some(11));
    assert_eq!(first.common_prefix_blocks, Some(2));
    assert_eq!(first.common_prefix_estimated_tokens, Some(5));
    assert_eq!(
        (
            first.repeated_blocks,
            first.changed_blocks,
            first.new_blocks,
            first.removed_blocks,
            first.common_prefix_blocks,
        ),
        (
            second.repeated_blocks,
            second.changed_blocks,
            second.new_blocks,
            second.removed_blocks,
            second.common_prefix_blocks,
        )
    );
}

#[test]
fn reorder_preserves_multiset_repetition_but_not_the_exact_prefix() {
    let previous = [
        block("a", Some("a"), Some(1)),
        block("a", Some("a"), Some(2)),
        block("b", Some("b"), Some(3)),
    ];
    let current = [
        block("b", Some("b"), Some(3)),
        block("a", Some("a"), Some(1)),
        block("a", Some("a"), Some(2)),
        block("a", Some("a"), Some(4)),
    ];

    let result = delta(&previous, &current, 32);
    assert_eq!(result.repeated_blocks, 3);
    assert_eq!(result.new_blocks, 1);
    assert_eq!(result.removed_blocks, 0);
    assert_eq!(result.common_prefix_blocks, Some(0));
}

#[test]
fn exact_duplicates_consume_the_semantically_corresponding_occurrence() {
    let previous = [
        block("same-raw", Some("first-role"), Some(1)),
        block("same-raw", Some("second-role"), Some(2)),
    ];
    let current = [
        block("same-raw", Some("second-role"), Some(2)),
        block("different-raw", Some("first-role"), Some(1)),
    ];

    let result = delta(&previous, &current, 32);
    assert_eq!(result.repeated_blocks, 1);
    assert_eq!(result.changed_blocks, 1);
    assert_eq!(result.new_blocks, 0);
    assert_eq!(result.removed_blocks, 0);
}

#[test]
fn missing_estimates_remain_unknown_for_their_aggregates() {
    let previous = [block("same", None, Some(1))];

    let current = [block("same", None, None), block("new", None, None)];

    let result = delta(&previous, &current, 32);
    assert_eq!(result.repeated_estimated_tokens, None);
    assert_eq!(result.new_estimated_tokens, None);
    assert_eq!(result.common_prefix_estimated_tokens, None);
}

#[test]
fn exact_cap_with_resource_status_is_not_reported_complete() {
    let previous = vec![block("same", None, Some(1)); 64];
    let current = vec![block("same", None, Some(1)); 64];

    let result = delta_with_status(DeltaInput {
        previous: &previous,
        current: &current,
        max_blocks: 64,
        previous_analysis_status: ContextAnalysisStatus::ResourceLimit,
        current_analysis_status: ContextAnalysisStatus::Complete,
    });

    assert_eq!(result.status, ContextDeltaStatus::ResourceLimit);
    assert_eq!(result.compared_previous_blocks, 64);
    assert_eq!(result.compared_current_blocks, 64);
    assert_eq!(result.common_prefix_blocks, None);
}

#[test]
fn inputs_over_the_block_bound_return_a_bounded_partial_delta() {
    let previous = vec![block("same", None, Some(1)); 10_000];
    let current = vec![block("same", None, Some(1)); 10_000];

    let result = delta(&previous, &current, 64);
    assert_eq!(result.status, ContextDeltaStatus::ResourceLimit);
    assert_eq!(result.compared_previous_blocks, 64);
    assert_eq!(result.compared_current_blocks, 64);
    assert_eq!(result.repeated_blocks, 64);
    assert_eq!(result.common_prefix_blocks, None);
}
