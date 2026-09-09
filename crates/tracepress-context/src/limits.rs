//! Explicit bounds for one shadow context analysis.

use std::num::NonZeroUsize;

use thiserror::Error;
use tracepress_core::{
    MaxCpuWorkUnits, MaxJsonNesting, MaxProcessingTimeMs, ProcessingBudget, ResourceLimits,
};

/// Names every independently bounded analysis dimension.
#[allow(
    clippy::exhaustive_enums,
    reason = "a limit failure must identify every analysis dimension explicitly"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextAnalysisLimitField {
    /// Request bytes admitted to analysis.
    AnalyzedBytes,
    /// Context blocks recorded for one snapshot.
    Blocks,
    /// JSON nesting depth the span indexer descends.
    JsonDepth,
    /// Bytes decoded from one JSON string.
    StringBytesInspected,
    /// CPU or abstract work units one analysis may charge.
    AnalysisWorkUnits,
    /// Wall-clock milliseconds one analysis may run.
    AnalysisWallTimeMs,
    /// IPC batches one snapshot may send.
    Batches,
}

/// Cohesive values used to construct validated analysis limits.
#[allow(
    clippy::exhaustive_structs,
    reason = "every analysis bound is deliberately required at the public validation boundary"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextAnalysisLimitValues {
    /// Request bytes admitted to analysis.
    pub max_analyzed_bytes: u64,
    /// Context blocks recorded for one snapshot.
    pub max_blocks: u64,
    /// JSON nesting depth the span indexer descends.
    pub max_json_depth: u64,
    /// Bytes decoded from one JSON string.
    pub max_string_bytes_inspected: u64,
    /// CPU or abstract work units one analysis may charge.
    pub max_analysis_work_units: u64,
    /// Wall-clock milliseconds one analysis may run.
    pub max_analysis_wall_time_ms: u64,
    /// IPC batches one snapshot may send.
    pub max_batches: u64,
}

/// Fully validated bounds for one shadow context analysis.
///
/// Crossing a bound stops that dimension of the work and reports
/// [`ContextAnalysisStatus::ResourceLimit`](crate::ContextAnalysisStatus::ResourceLimit); the
/// aggregates already completed inside the bound are kept, and the request is forwarded in full
/// either way.
///
/// The bounds are not a second set of globals: the byte bound and the work-unit bound are derived
/// from the configured [`ResourceLimits`] by [`ContextAnalysisLimits::from_resource_limits`], and
/// the remaining bounds are the fixed ceilings this analysis version declares.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ContextAnalysisLimits {
    /// Request bytes admitted to analysis.
    pub max_analyzed_bytes: NonZeroUsize,
    /// Context blocks recorded for one snapshot.
    pub max_blocks: NonZeroUsize,
    /// JSON nesting depth the span indexer descends.
    pub max_json_depth: MaxJsonNesting,
    /// Bytes decoded from one JSON string.
    pub max_string_bytes_inspected: NonZeroUsize,
    /// CPU or abstract work units one analysis may charge.
    pub max_analysis_work_units: MaxCpuWorkUnits,
    /// Wall-clock milliseconds one analysis may run.
    pub max_analysis_wall_time_ms: MaxProcessingTimeMs,
    /// IPC batches one snapshot may send.
    pub max_batches: NonZeroUsize,
}

impl ContextAnalysisLimits {
    /// Largest request prefix any configuration admits to analysis, in bytes.
    ///
    /// A larger request is still forwarded byte for byte; the difference is recorded as
    /// `skipped_bytes` rather than silently analyzed.
    pub const ANALYZED_BYTES_CEILING: u64 = 4 * 1024 * 1024;
    /// Context blocks one snapshot may record.
    pub const MAX_BLOCKS: u64 = 8192;
    /// JSON nesting depth the span indexer descends.
    pub const MAX_JSON_DEPTH: u64 = 64;
    /// Bytes decoded from one JSON string.
    pub const MAX_STRING_BYTES_INSPECTED: u64 = 65_536;
    /// Wall-clock milliseconds one analysis may run.
    pub const MAX_ANALYSIS_WALL_TIME_MS: u64 = 250;
    /// IPC batches one snapshot may send.
    pub const MAX_BATCHES: u64 = 128;
    /// Share of the configured CPU work-unit budget granted to analysis.
    ///
    /// Semantic provider observation already charges the same detached task, so analysis takes
    /// half of one operation's configured work units instead of introducing a larger bound of its
    /// own.
    pub const WORK_UNIT_SHARE_DIVISOR: u64 = 2;

    /// Validates explicit analysis bounds.
    ///
    /// # Errors
    /// Returns [`ContextAnalysisLimitsError::Zero`] for a zero bound, which would make an
    /// analysis vacuous rather than bounded, and
    /// [`ContextAnalysisLimitsError::Unrepresentable`] for a byte or count bound this platform
    /// cannot address.
    pub fn new(values: ContextAnalysisLimitValues) -> Result<Self, ContextAnalysisLimitsError> {
        Ok(Self {
            max_analyzed_bytes: addressable(
                values.max_analyzed_bytes,
                ContextAnalysisLimitField::AnalyzedBytes,
            )?,
            max_blocks: addressable(values.max_blocks, ContextAnalysisLimitField::Blocks)?,
            max_json_depth: MaxJsonNesting::new(values.max_json_depth).map_err(|_| {
                ContextAnalysisLimitsError::Zero {
                    field: ContextAnalysisLimitField::JsonDepth,
                }
            })?,
            max_string_bytes_inspected: addressable(
                values.max_string_bytes_inspected,
                ContextAnalysisLimitField::StringBytesInspected,
            )?,
            max_analysis_work_units: MaxCpuWorkUnits::new(values.max_analysis_work_units).map_err(
                |_| ContextAnalysisLimitsError::Zero {
                    field: ContextAnalysisLimitField::AnalysisWorkUnits,
                },
            )?,
            max_analysis_wall_time_ms: MaxProcessingTimeMs::new(values.max_analysis_wall_time_ms)
                .map_err(|_| ContextAnalysisLimitsError::Zero {
                field: ContextAnalysisLimitField::AnalysisWallTimeMs,
            })?,
            max_batches: addressable(values.max_batches, ContextAnalysisLimitField::Batches)?,
        })
    }

    /// Derives analysis bounds from the configured process resource limits.
    ///
    /// The byte bound is the smaller of the accepted request body bound and
    /// [`Self::ANALYZED_BYTES_CEILING`], so analysis never inspects bytes the proxy would not
    /// accept and never grows past the ceiling this analysis version declares. The work-unit
    /// bound is [`Self::WORK_UNIT_SHARE_DIVISOR`] of the configured CPU work units, and the wall
    /// clock is [`Self::MAX_ANALYSIS_WALL_TIME_MS`] clamped to the configured processing time, so
    /// analysis cannot outlive the operation budget it shares.
    ///
    /// # Errors
    /// Returns [`ContextAnalysisLimitsError::Unrepresentable`] when a configured byte bound
    /// cannot be addressed on this platform.
    pub fn from_resource_limits(
        limits: &ResourceLimits,
    ) -> Result<Self, ContextAnalysisLimitsError> {
        let configured_work_units = limits.max_cpu_work_units.get();
        let analysis_work_units = configured_work_units
            .checked_div(Self::WORK_UNIT_SHARE_DIVISOR)
            .unwrap_or(configured_work_units)
            .max(1);
        Self::new(ContextAnalysisLimitValues {
            max_analyzed_bytes: limits
                .max_request_body_bytes
                .get()
                .min(Self::ANALYZED_BYTES_CEILING),
            max_blocks: Self::MAX_BLOCKS,
            max_json_depth: Self::MAX_JSON_DEPTH,
            max_string_bytes_inspected: Self::MAX_STRING_BYTES_INSPECTED,
            max_analysis_work_units: analysis_work_units,
            max_analysis_wall_time_ms: Self::MAX_ANALYSIS_WALL_TIME_MS
                .min(limits.max_processing_time_ms.get()),
            max_batches: Self::MAX_BATCHES,
        })
    }

    /// Creates the unused processing budget one analysis charges.
    #[must_use]
    pub const fn processing_budget(self) -> ProcessingBudget {
        ProcessingBudget::new(self.max_analysis_wall_time_ms, self.max_analysis_work_units)
    }
}

/// Failure to validate explicit analysis bounds.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ContextAnalysisLimitsError {
    /// A bound was zero, which bounds nothing and analyzes nothing.
    #[error("analysis limit {field:?} must be greater than zero")]
    Zero {
        /// The rejected dimension.
        field: ContextAnalysisLimitField,
    },
    /// A bound could not be addressed on this platform.
    #[error("analysis limit {field:?} value {value} cannot be addressed on this platform")]
    Unrepresentable {
        /// The rejected dimension.
        field: ContextAnalysisLimitField,
        /// The rejected value.
        value: u64,
    },
}

fn addressable(
    value: u64,
    field: ContextAnalysisLimitField,
) -> Result<NonZeroUsize, ContextAnalysisLimitsError> {
    let addressable = usize::try_from(value)
        .map_err(|_| ContextAnalysisLimitsError::Unrepresentable { field, value })?;
    NonZeroUsize::new(addressable).ok_or(ContextAnalysisLimitsError::Zero { field })
}
