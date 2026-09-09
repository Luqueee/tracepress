#![no_main]

//! Fuzzes the bounded non-streaming Responses v1 response parser, including isolation of the
//! provider `usage` object, so retained usage stays inside the configured byte bound and is never
//! sized from a length declared in the input.
//!
//! The whole input is the response body: the content-capture policy is derived from its length, so
//! a seeded JSON corpus stays parseable while every policy is still reached.

mod common;

use std::hint::black_box;

use libfuzzer_sys::fuzz_target;
use tracepress_provider::{
    normalize_usage, parse_response, ContentCaptureMode, ObservationInput,
    OpenAiResponsesV1Observer, ProviderObserver, ProviderResponseState, UsageStatus,
};

/// Derives the content-capture policy from the body length.
fn capture_mode(length: usize) -> ContentCaptureMode {
    match length % 3 {
        1 => ContentCaptureMode::Off,
        2 => ContentCaptureMode::LocalRaw,
        _ => ContentCaptureMode::MetadataOnly,
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(limits) = common::limits() else {
        return;
    };
    let input = ObservationInput::new(data, limits).with_content_capture(capture_mode(data.len()));
    let observation = parse_response(input);
    if let Some(raw) = observation.raw_usage.as_ref() {
        // Isolated usage is a bounded copy of bytes that were actually present, and it
        // re-normalizes to exactly what the parse reported.
        assert!(
            raw.len() <= limits.max_usage_bytes,
            "retained usage must stay inside the configured byte bound"
        );
        assert!(
            raw.len() <= data.len(),
            "retained usage cannot exceed the bytes presented to the parser"
        );
        let renormalized = normalize_usage(raw);
        assert_eq!(
            observation.normalized_usage.as_ref(),
            Some(&renormalized),
            "normalization must be a pure function of the retained usage bytes"
        );
    } else {
        assert_ne!(
            observation.usage_status,
            UsageStatus::Final,
            "usage cannot be final without a retained usage object"
        );
    }
    // Final usage requires terminal provider evidence.
    if observation.usage_status == UsageStatus::Final {
        assert!(
            matches!(
                observation.response_state,
                ProviderResponseState::Completed
                    | ProviderResponseState::Incomplete
                    | ProviderResponseState::Failed
                    | ProviderResponseState::Cancelled
            ),
            "final usage requires a terminal response state, found {:?}",
            observation.response_state
        );
    }
    // An offline parse reads no clock, so it never invents a local measurement.
    assert_eq!(observation.ttfb_us, None);
    assert_eq!(observation.ttft_us, None);
    assert_eq!(observation.duration_us, None);

    let observer = OpenAiResponsesV1Observer::new();
    match observer.observe_response(input) {
        Ok(observed) => {
            assert_eq!(
                observed, observation,
                "the observer capability must agree with the free function"
            );
        }
        Err(error) => {
            black_box(error);
        }
    }
});
