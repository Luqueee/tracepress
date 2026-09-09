#![no_main]

//! Fuzzes usage retention and normalization. The whole input is offered as a provider `usage`
//! object twice: once under the configured finite bound, exercising retention and normalization,
//! and once under a tighter bound derived from the input, exercising the rejection path. Nothing
//! is ever allocated from a length declared inside the input.

mod common;

use std::hint::black_box;

use libfuzzer_sys::fuzz_target;
use tracepress_provider::{
    normalize_usage, NormalizedUsage, ObservationLimits, RawProviderUsage, UsageError, UsageStatus,
};

/// Scale applied to the input-derived byte bound.
const BOUND_GRANULARITY: usize = 64;

/// Derives a tighter usage bound from the input, never above the configured limit.
fn tighter_bound(data: &[u8], limits: ObservationLimits) -> usize {
    let selector = usize::from(data.last().copied().unwrap_or_default());
    selector
        .saturating_mul(BOUND_GRANULARITY)
        .min(limits.max_usage_bytes)
}

/// Reports whether normalized components stayed consistent or were flagged as anomalous.
fn consistent(usage: &NormalizedUsage) -> bool {
    let input_ok = match (usage.input_total, usage.input_cached) {
        (Some(total), Some(cached)) => cached <= total,
        _ => true,
    };
    let output_ok = match (usage.output_total, usage.output_reasoning) {
        (Some(total), Some(reasoning)) => reasoning <= total,
        _ => true,
    };
    (input_ok && output_ok) || usage.anomalies.any()
}

/// Reports whether every component stayed inside the durable signed 64-bit counter range.
fn durable(usage: &NormalizedUsage) -> bool {
    [
        usage.input_total,
        usage.input_cached,
        usage.input_uncached,
        usage.cache_write,
        usage.output_total,
        usage.output_reasoning,
        usage.total,
    ]
    .iter()
    .all(|component| component.is_none_or(|value| value <= i64::MAX as u64))
}

/// Reports whether any component carries a value at all.
fn reported(usage: &NormalizedUsage) -> bool {
    [
        usage.input_total,
        usage.input_cached,
        usage.cache_write,
        usage.output_total,
        usage.output_reasoning,
        usage.total,
    ]
    .iter()
    .any(Option::is_some)
}

fuzz_target!(|data: &[u8]| {
    let Ok(limits) = common::limits() else {
        return;
    };
    match RawProviderUsage::new(data, limits.max_usage_bytes) {
        Ok(raw) => {
            // Accepted bytes are an exact copy, never a reallocation from a declared length.
            assert_eq!(
                raw.as_bytes(),
                data,
                "retained usage must be an exact copy of the accepted bytes"
            );
            assert!(
                !raw.is_empty(),
                "an accepted usage object always carries its bytes"
            );
            let normalized = normalize_usage(&raw);
            // Normalization is fail-open: an unusable component is absent or flagged, never
            // fabricated above its reported total.
            assert!(
                consistent(&normalized),
                "inconsistent usage must be flagged rather than reported as clean"
            );
            // No provider-chosen number may produce a component a durable counter cannot hold.
            assert!(
                durable(&normalized),
                "a component exceeded the durable signed 64-bit range"
            );
            // Status is evidence-backed: nothing is final without a value.
            assert_eq!(
                normalized.status == UsageStatus::Unavailable,
                !reported(&normalized),
                "usage status must follow the components actually extracted"
            );
            black_box(normalized);
        }
        Err(UsageError::ResourceLimit) => {
            assert!(
                data.len() > limits.max_usage_bytes,
                "usage bytes are only rejected for length once they exceed the bound"
            );
        }
        Err(error) => {
            black_box(error);
        }
    }

    let bound = tighter_bound(data, limits);
    match RawProviderUsage::new(data, bound) {
        Ok(raw) => {
            assert!(
                raw.len() <= bound,
                "retained usage must respect the tighter bound"
            );
            black_box(normalize_usage(&raw));
        }
        Err(error) => {
            black_box(error);
        }
    }
});
