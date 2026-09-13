#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_compression::{DetectedKind, JsonTabular};

fuzz_target!(|data: &[u8]| common::exercise(&JsonTabular, data, DetectedKind::Json));
