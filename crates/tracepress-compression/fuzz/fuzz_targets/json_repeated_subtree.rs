#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_compression::{DetectedKind, JsonRepeatedSubtree};

fuzz_target!(|data: &[u8]| common::exercise(&JsonRepeatedSubtree, data, DetectedKind::Json));
