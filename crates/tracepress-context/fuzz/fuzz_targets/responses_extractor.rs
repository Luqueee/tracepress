#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_context::{ContextAnalysisStatus, ContextDigest, analyze_responses};

fuzz_target!(|data: &[u8]| {
    let limits = common::limits();
    let first = analyze_responses(data, limits);
    let second = analyze_responses(data, limits);

    // Extraction is deterministic metadata over the original request, never an in-place rewrite.
    assert_eq!(first, second);
    assert_eq!(first.request_content_hash, ContextDigest::from_bytes(data));
    assert_eq!(
        first.analyzed_bytes.saturating_add(first.skipped_bytes),
        common::as_u64(data.len())
    );
    assert!(first.blocks.len() <= limits.max_blocks.get());
    assert!(matches!(
        first.status,
        ContextAnalysisStatus::Complete
            | ContextAnalysisStatus::Partial
            | ContextAnalysisStatus::ResourceLimit
            | ContextAnalysisStatus::Malformed
    ));
    if first.status == ContextAnalysisStatus::ResourceLimit {
        assert!(first.reason.is_some());
    }

    for block in &first.blocks {
        assert_eq!(
            block.raw_bytes,
            block
                .locator
                .raw_value_end
                .saturating_sub(block.locator.raw_value_start)
        );
        let span = tracepress_context::RawSpan::new(
            block.locator.raw_value_start,
            block.locator.raw_value_end,
        )
        .expect("extractor emits ordered spans");
        let bytes = span.slice(data).expect("block span remains inside request");
        assert_eq!(block.exact_fingerprint, ContextDigest::from_bytes(bytes));
    }
});
