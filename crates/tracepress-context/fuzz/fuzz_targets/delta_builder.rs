#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_context::{
    ContextAnalysisLimitValues, ContextAnalysisLimits, ContextBlockKind, ContextBlockSummary,
    ContextDeltaRequest, ContextDeltaStatus, ContextDigest, ContextRole, JsonValueKind, RawSpan,
    SEMANTIC_FINGERPRINT_VERSION, SemanticFingerprintInput, compute_context_delta,
    semantic_fingerprint,
};
use tracepress_core::{ContextSnapshotId, UuidV7Generator};

fn delta_limits() -> ContextAnalysisLimits {
    ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes: 64 * 1024,
        max_blocks: 16,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: 4_096,
        max_analysis_work_units: 100_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })
    .expect("delta fuzz limits are valid")
}

fn semantic(index: usize, limits: ContextAnalysisLimits) -> tracepress_context::SemanticFingerprint {
    let encoded = format!(r#""fuzz-{index}""#);
    semantic_fingerprint(SemanticFingerprintInput {
        request: encoded.as_bytes(),
        block_kind: ContextBlockKind::Text,
        role: ContextRole::User,
        value_span: RawSpan::new(0, common::as_u64(encoded.len())).expect("semantic span"),
        value_kind: JsonValueKind::String,
        duplicate_key_in_subtree: false,
        fingerprint_version: SEMANTIC_FINGERPRINT_VERSION,
        limits,
    })
    .expect("bounded semantic fixture is valid")
}

fn summaries(data: &[u8], limits: ContextAnalysisLimits, reverse: bool) -> Vec<ContextBlockSummary> {
    let mut result: Vec<_> = data
        .chunks(4)
        .take(128)
        .enumerate()
        .map(|(index, bytes)| ContextBlockSummary {
            exact_fingerprint: ContextDigest::from_bytes(bytes),
            semantic_fingerprint: (index % 3 == 0).then(|| semantic(index, limits)),
            estimated_tokens: (index % 5 != 0).then_some(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
        })
        .collect();
    if reverse {
        result.reverse();
    }
    result
}

fuzz_target!(|data: &[u8]| {
    let limits = delta_limits();
    let previous = summaries(data, limits, false);
    let current = summaries(data, limits, true);
    let generator = UuidV7Generator::new();
    let previous_snapshot_id = ContextSnapshotId::generate(&generator);
    let current_snapshot_id = ContextSnapshotId::generate(&generator);
    let request = ContextDeltaRequest {
        previous_snapshot_id,
        current_snapshot_id,
        previous: &previous,
        current: &current,
        limits,
    };

    let first = compute_context_delta(request);
    let second = compute_context_delta(request);
    assert_eq!(first, second);
    assert!(first.compared_previous_blocks <= limits.max_blocks.get() as u64);
    assert!(first.compared_current_blocks <= limits.max_blocks.get() as u64);
    assert_eq!(
        first.repeated_blocks + first.new_blocks + first.changed_blocks,
        first.compared_current_blocks
    );
    assert!(first.removed_blocks <= first.compared_previous_blocks);
    let was_limited = previous.len() > limits.max_blocks.get() || current.len() > limits.max_blocks.get();
    assert_eq!(first.status, if was_limited {
        ContextDeltaStatus::ResourceLimit
    } else {
        ContextDeltaStatus::Complete
    });
    if !was_limited {
        assert_eq!(first.compared_previous_blocks, common::as_u64(previous.len()));
        assert_eq!(first.compared_current_blocks, common::as_u64(current.len()));
    }
});
