#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_compression::{DetectedKind, JsonMinify};

fuzz_target!(|data: &[u8]| common::exercise(&JsonMinify, data, DetectedKind::Json));
