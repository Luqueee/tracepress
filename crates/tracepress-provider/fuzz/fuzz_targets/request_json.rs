#![no_main]

//! Fuzzes the bounded Responses v1 request parser through both the free function and the public
//! observer capability, so deep nesting, duplicate keys, oversized strings, and oversized bodies
//! are rejected fail-open instead of retained.
//!
//! The whole input is the request body: the content-capture policy is derived from its length, so
//! a seeded JSON corpus stays parseable while every policy is still reached.

mod common;

use std::hint::black_box;

use libfuzzer_sys::fuzz_target;
use tracepress_provider::{
    parse_request, ContentCaptureMode, ObservationInput, OpenAiResponsesV1Observer,
    ProviderObserver,
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
    let observation = parse_request(input);
    // A parse must never claim more request bytes than were presented, and no retained scalar may
    // exceed the configured string bound.
    assert_eq!(
        observation.request_bytes,
        u64::try_from(data.len()).ok(),
        "request bytes must report exactly the presented length"
    );
    for (field, value) in [
        ("model", observation.model.as_ref()),
        ("reasoning_effort", observation.reasoning_effort.as_ref()),
        ("verbosity", observation.verbosity.as_ref()),
        ("truncation", observation.truncation.as_ref()),
    ] {
        assert!(
            value.is_none_or(|text| text.len() <= limits.max_string_bytes),
            "retained {field} exceeded the configured string bound"
        );
        assert!(
            value.is_none_or(|text| !text.as_bytes().contains(&0)),
            "retained {field} carried a NUL byte"
        );
    }

    let observer = OpenAiResponsesV1Observer::new();
    black_box(observer.provider());
    black_box(observer.protocol());
    match observer.observe_request(input) {
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
