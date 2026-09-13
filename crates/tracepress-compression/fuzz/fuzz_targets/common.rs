use tracepress_compression::{
    BlockKind, BlockMetadata, BlockOrigin, CompressionLimits, DetectedKind, ShadowCompressor,
    evaluate,
};

pub fn exercise(compressor: &dyn ShadowCompressor, data: &[u8], detected: DetectedKind) {
    let mut limits = CompressionLimits::default();
    limits.max_candidate_input_bytes = 65_536;
    limits.max_candidate_output_bytes = 65_536;
    limits.max_shadow_memory_bytes = 262_144;
    limits.max_shadow_work_units = 131_072;
    let metadata = BlockMetadata::new(
        BlockOrigin::ToolGenerated,
        BlockKind::ToolResult,
        detected,
        None,
        0,
        u64::try_from(data.len()).unwrap_or(u64::MAX),
        None,
        None,
    );
    let first = evaluate(compressor, metadata, data, &limits);
    let second = evaluate(compressor, metadata, data, &limits);
    assert_eq!(first.metrics(), second.metrics());
}
