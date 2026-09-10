#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_context::{
    ContextBlockKind, ContextRole, JsonValueKind, RawSpan, SEMANTIC_FINGERPRINT_VERSION,
    SemanticFingerprintInput, exact_fingerprint, semantic_fingerprint,
};

const MAX_SOURCE_BYTES: usize = 4_096;
const HEX: &[u8; 16] = b"0123456789abcdef";

fn escaped_json_string(data: &[u8]) -> Vec<u8> {
    let source = data.get(..data.len().min(MAX_SOURCE_BYTES)).unwrap_or(data);
    let mut encoded = Vec::with_capacity(source.len().saturating_mul(6).saturating_add(2));
    encoded.push(b'"');
    for byte in source {
        encoded.extend_from_slice(br#"\u00"#);
        encoded.push(HEX[usize::from(byte >> 4)]);
        encoded.push(HEX[usize::from(byte & 0x0f)]);
    }
    encoded.push(b'"');
    encoded
}

fuzz_target!(|data: &[u8]| {
    let request = escaped_json_string(data);
    let span = RawSpan::new(0, common::as_u64(request.len())).expect("encoded span is ordered");
    let limits = common::limits();
    let input = SemanticFingerprintInput {
        request: &request,
        block_kind: ContextBlockKind::Text,
        role: ContextRole::User,
        value_span: span,
        value_kind: JsonValueKind::String,
        duplicate_key_in_subtree: false,
        fingerprint_version: SEMANTIC_FINGERPRINT_VERSION,
        limits,
    };

    let first = semantic_fingerprint(input);
    let second = semantic_fingerprint(input);
    assert_eq!(first, second);
    assert!(exact_fingerprint(&request, span).is_some());
    let source_len = data.len().min(MAX_SOURCE_BYTES);
    if u64::try_from(source_len).unwrap_or(u64::MAX)
        <= common::as_u64(limits.max_string_bytes_inspected.get()) / 2
    {
        assert!(first.is_some());
    }

    let mut duplicate = input;
    duplicate.duplicate_key_in_subtree = true;
    assert!(semantic_fingerprint(duplicate).is_none());

    // Arbitrary bytes are also a valid total-input exercise for the decoder path: malformed
    // strings are refused, never normalized or allowed to panic.
    let raw_len = data.len().min(MAX_SOURCE_BYTES);
    let raw_span = RawSpan::new(0, common::as_u64(raw_len)).expect("raw span is ordered");
    let raw_input = SemanticFingerprintInput {
        request: data,
        value_span: raw_span,
        ..input
    };
    let _ = semantic_fingerprint(raw_input);
});
