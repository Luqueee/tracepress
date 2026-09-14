//! Metadata-only offline evaluator for controlled public tool workloads.
//!
//! The command reads one bounded `ToolResult` from stdin, applies a named shadow reducer, and emits
//! only allowlisted metrics. It exists for Phase 4.5 public workload characterization; it never
//! forwards, persists, or prints the input/output content.

use std::io::{self, Read, Write};

use serde::Serialize;
use tracepress_compression::{
    BlockKind, BlockMetadata, BlockOrigin, CompressionLimits, DetectedKind, ReductionStatus,
    SearchResultModel, SearchResultReducer, ShellSemanticFamily, ToolResultReducer,
};

const MAX_INPUT_BYTES: usize = 1_048_576;

#[derive(Serialize)]
struct ShadowOutput {
    family: &'static str,
    reducer: &'static str,
    status: ReductionStatus,
    input_bytes: u64,
    output_bytes: Option<u64>,
    byte_reduction: Option<u64>,
    recovery_verified: bool,
    deterministic: bool,
    canonical_equal: Option<bool>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let family = std::env::args().nth(1).unwrap_or_default();
    if family != "search" {
        return Err("only the bounded search shadow is enabled".into());
    }
    let mut input = Vec::new();
    let _bytes_read = io::stdin()
        .take(
            u64::try_from(MAX_INPUT_BYTES)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut input)?;
    if input.len() > MAX_INPUT_BYTES {
        return Err("input exceeds bounded public-workload shadow limit".into());
    }
    let limits = CompressionLimits::default();
    let metadata = BlockMetadata::new(
        BlockOrigin::ToolGenerated,
        BlockKind::ToolResult,
        DetectedKind::PlainText,
        None,
        0,
        u64::try_from(input.len()).unwrap_or(u64::MAX),
        None,
        None,
    );
    let reducer = SearchResultReducer;
    assert!(reducer.supports(metadata));
    let first = reducer.reduce(&input, &limits);
    let second = reducer.reduce(&input, &limits);
    let deterministic = first.metrics().visible_fingerprint == second.metrics().visible_fingerprint
        && first.metrics().status == second.metrics().status;
    let canonical_equal = first.visible().and_then(|visible| {
        let original = SearchResultModel::parse(&input, &limits)?;
        let projected = SearchResultModel::parse(visible, &limits)?;
        Some(original.matches == projected.matches)
    });
    let metrics = first.with_deterministic(deterministic).metrics().clone();
    let result = ShadowOutput {
        family: ShellSemanticFamily::Search.as_str(),
        reducer: metrics.reducer_id,
        status: metrics.status,
        input_bytes: metrics.input_bytes,
        output_bytes: metrics.visible_bytes,
        byte_reduction: metrics.gross_bytes_delta,
        recovery_verified: metrics.recovery_verified,
        deterministic: metrics.deterministic,
        canonical_equal,
    };
    writeln!(io::stdout(), "{}", serde_json::to_string(&result)?)?;
    Ok(())
}
