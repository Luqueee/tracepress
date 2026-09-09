//! Construction and derivation contract of the analysis bounds.

use tracepress_context::{
    ContextAnalysisLimitField, ContextAnalysisLimitValues, ContextAnalysisLimits,
    ContextAnalysisLimitsError,
};
use tracepress_core::{ResourceLimits, ResourceLimitsConfig, ResourceLimitsError};

type SetterRow = (
    ContextAnalysisLimitField,
    fn(&mut ContextAnalysisLimitValues),
);

const ZERO_ROWS: [SetterRow; 7] = [
    (ContextAnalysisLimitField::AnalyzedBytes, |values| {
        values.max_analyzed_bytes = 0;
    }),
    (ContextAnalysisLimitField::Blocks, |values| {
        values.max_blocks = 0;
    }),
    (ContextAnalysisLimitField::JsonDepth, |values| {
        values.max_json_depth = 0;
    }),
    (ContextAnalysisLimitField::StringBytesInspected, |values| {
        values.max_string_bytes_inspected = 0;
    }),
    (ContextAnalysisLimitField::AnalysisWorkUnits, |values| {
        values.max_analysis_work_units = 0;
    }),
    (ContextAnalysisLimitField::AnalysisWallTimeMs, |values| {
        values.max_analysis_wall_time_ms = 0;
    }),
    (ContextAnalysisLimitField::Batches, |values| {
        values.max_batches = 0;
    }),
];

#[test]
fn a_zero_bound_is_rejected_and_names_its_dimension() {
    for (field, set_zero) in ZERO_ROWS {
        // Given
        let mut values = valid_values();
        set_zero(&mut values);

        // When
        let result = ContextAnalysisLimits::new(values);

        // Then
        assert_eq!(
            result.err(),
            Some(ContextAnalysisLimitsError::Zero { field }),
            "a zero {field:?} bound must be rejected"
        );
    }
}

#[test]
fn analyzed_bytes_never_exceed_the_declared_ceiling() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let resource_limits = resource_limits(8 * 1024 * 1024, 1_000, 4_000)?;

    // When
    let limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)?;

    // Then
    assert_eq!(
        u64::try_from(limits.max_analyzed_bytes.get())?,
        ContextAnalysisLimits::ANALYZED_BYTES_CEILING
    );
    Ok(())
}

#[test]
fn analyzed_bytes_never_exceed_the_accepted_request_body() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let accepted_body_bytes = 1024 * 1024;
    let resource_limits = resource_limits(accepted_body_bytes, 1_000, 4_000)?;

    // When
    let limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)?;

    // Then
    assert_eq!(
        u64::try_from(limits.max_analyzed_bytes.get())?,
        accepted_body_bytes
    );
    Ok(())
}

#[test]
fn analysis_takes_a_share_of_the_configured_work_units() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let configured_work_units = 4_000;
    let resource_limits = resource_limits(1024 * 1024, 1_000, configured_work_units)?;

    // When
    let limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)?;

    // Then
    assert_eq!(
        limits.max_analysis_work_units.get(),
        configured_work_units / ContextAnalysisLimits::WORK_UNIT_SHARE_DIVISOR
    );
    Ok(())
}

#[test]
fn a_minimal_work_unit_budget_still_permits_one_unit() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let resource_limits = resource_limits(1024 * 1024, 1_000, 1)?;

    // When
    let limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)?;

    // Then
    assert_eq!(limits.max_analysis_work_units.get(), 1);
    Ok(())
}

#[test]
fn analysis_never_outlives_the_configured_processing_time() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let configured_processing_time_ms = 40;
    let resource_limits = resource_limits(1024 * 1024, configured_processing_time_ms, 4_000)?;

    // When
    let limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)?;

    // Then
    assert_eq!(
        limits.max_analysis_wall_time_ms.get(),
        configured_processing_time_ms
    );
    Ok(())
}

#[test]
fn a_generous_processing_time_is_capped_by_the_analysis_ceiling()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let resource_limits = resource_limits(1024 * 1024, 60_000, 4_000)?;

    // When
    let limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)?;

    // Then
    assert_eq!(
        limits.max_analysis_wall_time_ms.get(),
        ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS
    );
    Ok(())
}

const fn valid_values() -> ContextAnalysisLimitValues {
    ContextAnalysisLimitValues {
        max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
        max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        max_analysis_work_units: 2_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    }
}

fn resource_limits(
    max_request_body_bytes: u64,
    max_processing_time_ms: u64,
    max_cpu_work_units: u64,
) -> Result<ResourceLimits, ResourceLimitsError> {
    ResourceLimits::try_from(ResourceLimitsConfig {
        max_raw_bytes: Some(1024 * 1024),
        max_request_body_bytes: Some(i128::from(max_request_body_bytes)),
        max_response_body_bytes: Some(32 * 1024 * 1024),
        max_decompressed_bytes: Some(32 * 1024 * 1024),
        max_ipc_frame_bytes: Some(65_536),
        max_ipc_queue_items: Some(1_024),
        max_json_nesting: Some(64),
        max_json_items: Some(100_000),
        max_line_bytes: Some(65_536),
        max_processing_time_ms: Some(i128::from(max_processing_time_ms)),
        max_cpu_work_units: Some(i128::from(max_cpu_work_units)),
    })
}
