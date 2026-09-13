#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_compression::{DetectedKind, TextRepeatedRun};

fuzz_target!(|data: &[u8]| {
    common::exercise(&TextRepeatedRun, data, DetectedKind::PlainText);
});
