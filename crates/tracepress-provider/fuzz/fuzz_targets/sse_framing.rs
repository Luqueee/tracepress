#![no_main]

//! Fuzzes bounded SSE framing: the input carries a fragment-width schedule followed by an intact
//! stream body, which is fed to [`SseFramer`] incrementally at those fuzzer-chosen widths, so
//! line, event-byte, and event-count bounds are crossed at arbitrary chunk boundaries.

mod chunking;
mod common;

use std::hint::black_box;

use libfuzzer_sys::fuzz_target;
use tracepress_provider::SseFramer;

fuzz_target!(|data: &[u8]| {
    let Ok(limits) = common::limits() else {
        return;
    };
    let Some((schedule, body)) = chunking::split_schedule(data) else {
        return;
    };
    let mut framer = SseFramer::new(limits);
    let mut framed_data_bytes = 0_usize;
    let mut offset = 0_usize;
    let mut step = 0_usize;
    // Widths are at least one byte, so `offset` strictly grows and this terminates.
    while offset < body.len() {
        let end = offset
            .saturating_add(chunking::width_at(schedule, step))
            .min(body.len());
        let Some(chunk) = body.get(offset..end) else {
            return;
        };
        offset = end;
        step = step.saturating_add(1);
        match framer.push(chunk) {
            Ok(events) => {
                for event in &events {
                    framed_data_bytes = framed_data_bytes.saturating_add(event.data.len());
                    // No event may carry more data than the configured per-event bound.
                    assert!(
                        event.data.len() <= limits.max_sse_event_bytes,
                        "event data {} exceeds the configured per-event bound {}",
                        event.data.len(),
                        limits.max_sse_event_bytes
                    );
                }
                assert!(
                    framer.event_count() <= limits.max_sse_events,
                    "framer emitted more events than the configured bound"
                );
                black_box(events);
            }
            Err(error) => {
                // A framing bound was exceeded; the framer stays failed, so stop feeding it.
                black_box(error);
                black_box(framed_data_bytes);
                assert!(
                    framer.event_count() <= limits.max_sse_events,
                    "framer emitted more events than the configured bound"
                );
                return;
            }
        }
    }
    match framer.finish() {
        Ok(events) => {
            for event in &events {
                framed_data_bytes = framed_data_bytes.saturating_add(event.data.len());
                assert!(
                    event.data.len() <= limits.max_sse_event_bytes,
                    "flushed event data exceeds the configured per-event bound"
                );
            }
            black_box(events);
        }
        Err(error) => {
            black_box(error);
        }
    }
    black_box(framed_data_bytes);
    assert!(
        framer.event_count() <= limits.max_sse_events,
        "framer emitted more events than the configured bound"
    );
});
