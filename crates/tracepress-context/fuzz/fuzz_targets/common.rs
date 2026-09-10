#![allow(dead_code)]

use std::convert::TryFrom;

use tracepress_context::{ContextAnalysisLimitValues, ContextAnalysisLimits};

pub const INPUT_PREFIX_BYTES: usize = 64 * 1024;

pub fn limits() -> ContextAnalysisLimits {
    ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes: u64::try_from(INPUT_PREFIX_BYTES).expect("prefix fits u64"),
        max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: 4_096,
        max_analysis_work_units: 100_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })
    .expect("fuzz limits are valid")
}

pub fn observed_prefix(data: &[u8]) -> &[u8] {
    data.get(..data.len().min(INPUT_PREFIX_BYTES)).unwrap_or(data)
}

pub fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
