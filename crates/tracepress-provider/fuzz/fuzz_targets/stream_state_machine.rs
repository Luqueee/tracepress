#![no_main]

//! Fuzzes the streaming Responses lifecycle state machine: the input carries a terminal-decision
//! selector, a fragment-width schedule, and an intact stream body. The body is pushed to
//! [`StreamingObserver`] at those fuzzer-chosen widths and the stream is always driven to one
//! terminal decision, after which further fragments must stay ignored and the summary must not
//! move.

mod chunking;
mod common;

use std::hint::black_box;

use libfuzzer_sys::fuzz_target;
use tracepress_provider::{
    ObservationLimits, ProviderResponseState, ResponseObservation, StreamingObserver, UsageStatus,
};

/// Terminal decision taken by the stream owner.
enum Terminal {
    /// The stream ended cleanly.
    Finish,
    /// The client cancelled.
    Cancel,
    /// The upstream disconnected.
    Disconnect,
}

/// Derives the terminal decision from one input byte.
fn terminal(selector: u8) -> Terminal {
    match selector % 3 {
        1 => Terminal::Cancel,
        2 => Terminal::Disconnect,
        _ => Terminal::Finish,
    }
}

/// Feeds the observer at the scheduled widths and returns the number of bytes it accepted.
fn drive(observer: &mut StreamingObserver, schedule: &[u8], body: &[u8]) -> usize {
    let mut accepted = 0_usize;
    let mut offset = 0_usize;
    let mut step = 0_usize;
    // Widths are at least one byte, so `offset` strictly grows and this terminates.
    while offset < body.len() {
        let end = offset
            .saturating_add(chunking::width_at(schedule, step))
            .min(body.len());
        let Some(chunk) = body.get(offset..end) else {
            return accepted;
        };
        offset = end;
        step = step.saturating_add(1);
        if let Err(error) = observer.push(chunk) {
            // A framing bound was exceeded; the observer keeps its bounded status, so stop.
            black_box(error);
            return accepted;
        }
        accepted = accepted.saturating_add(chunk.len());
    }
    accepted
}

/// Reports whether retained usage stayed inside the configured finite bound.
fn usage_within_bound(observation: &ResponseObservation, limits: ObservationLimits) -> bool {
    observation
        .raw_usage
        .as_ref()
        .is_none_or(|usage| usage.len() <= limits.max_usage_bytes)
}

fuzz_target!(|data: &[u8]| {
    let Ok(limits) = common::limits() else {
        return;
    };
    let Some((&decision, rest)) = data.split_first() else {
        return;
    };
    let Some((schedule, body)) = chunking::split_schedule(rest) else {
        return;
    };
    let mut observer = StreamingObserver::new(limits);
    black_box(drive(&mut observer, schedule, body));
    black_box(observer.summary());
    let observation = match terminal(decision) {
        Terminal::Finish => observer.finish(),
        Terminal::Cancel => observer.cancel(),
        Terminal::Disconnect => observer.disconnect(),
    };
    assert!(
        usage_within_bound(&observation, limits),
        "retained usage must stay inside the configured byte bound"
    );
    // Final usage requires terminal provider evidence, never a client or transport decision.
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
    // Timing evidence is measured, never fabricated, so it can only appear in order.
    if let (Some(first_byte), Some(first_token)) = (observation.ttfb_us, observation.ttft_us) {
        assert!(
            first_byte <= first_token,
            "time to first token must not precede the first upstream byte"
        );
    }
    if let (Some(first_byte), Some(duration)) = (observation.ttfb_us, observation.duration_us) {
        assert!(
            first_byte <= duration,
            "duration must not precede the first upstream byte"
        );
    }
    // A terminal observer must ignore further fragments, so its summary must still equal the
    // terminal observation.
    if let Err(error) = observer.observe_chunk(body) {
        black_box(error);
    }
    assert_eq!(
        observer.summary(),
        observation,
        "a terminal observer must not mutate its observation"
    );
    black_box(observation);
});
