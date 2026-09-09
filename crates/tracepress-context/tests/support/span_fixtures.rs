// Shared span fixtures. The including test target owns the imports of `AnalysisClock`,
// `ContextAnalysisLimitValues`, `ContextAnalysisLimits`, `ContextAnalysisLimitsError`, and
// `RawSpanIndex`.

/// A clock that never advances, so a bounded scan is deterministic in a test.
struct FrozenClock;

impl AnalysisClock for FrozenClock {
    fn elapsed_ms(&self) -> u64 {
        0
    }
}

/// The base bounds a test adjusts one dimension of.
const fn analysis_values() -> ContextAnalysisLimitValues {
    ContextAnalysisLimitValues {
        max_analyzed_bytes: 4 * 1024 * 1024,
        max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        max_analysis_work_units: 1_000_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    }
}

/// The bounds a request of realistic size is analysed under.
fn generous_limits() -> Result<ContextAnalysisLimits, ContextAnalysisLimitsError> {
    ContextAnalysisLimits::new(analysis_values())
}

/// Indexes one document under generous bounds and a frozen clock.
fn index_document(document: &str) -> Result<RawSpanIndex, ContextAnalysisLimitsError> {
    Ok(RawSpanIndex::build_with_clock(
        document.as_bytes(),
        generous_limits()?,
        &FrozenClock,
    ))
}
