#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_compression::{DetectedKind, TextRepeatedLine};

fuzz_target!(|data: &[u8]| {
    common::exercise(&TextRepeatedLine, data, DetectedKind::PlainText);
});
