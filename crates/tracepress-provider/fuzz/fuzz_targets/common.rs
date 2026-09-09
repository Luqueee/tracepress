//! Explicit finite observation limits shared by every provider fuzz target.

use tracepress_provider::{LimitsError, ObservationLimitValues, ObservationLimits};

/// Explicit finite observation limits.
///
/// Every bound is deliberately small so a short fuzz input can reach it, and none is zero, so
/// construction succeeds without falling back to the permissive defaults.
///
/// # Errors
///
/// Returns [`LimitsError`] when a bound is rejected by the provider crate.
pub fn limits() -> Result<ObservationLimits, LimitsError> {
    ObservationLimits::new(ObservationLimitValues {
        max_semantic_bytes: 64 * 1024,
        max_usage_bytes: 8 * 1024,
        max_sse_event_bytes: 8 * 1024,
        max_sse_events: 128,
        max_json_depth: 16,
        max_json_items: 512,
        max_string_bytes: 512,
    })
}
