//! Opaque byte and streaming boundary contract tests.

use tracepress_core::{
    ByteBoundaryError, ByteDecision, ByteResource, DeclaredLengthDecision, MaxDecompressedBytes,
    MaxIpcFrameBytes, MaxLineBytes, MaxRawBytes, MaxRequestBodyBytes, MaxResponseBodyBytes,
    ResponseLimitReason, StreamDecision, StreamingResponseBudget, classify_decompressed_length,
    classify_ipc_frame, classify_line_bytes, classify_raw_bytes, classify_request_body,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn arbitrary_opaque_bytes_within_limit_are_never_decoded_or_mutated() -> TestResult {
    // Given
    let bytes = [0xff, 0xfe, 0x00, 0x1b, b'[', b'3', b'1', b'm'];

    // When
    let decision = classify_raw_bytes(&bytes, MaxRawBytes::new(64)?)?;

    // Then
    assert_eq!(decision, ByteDecision::WithinLimit { bytes: &bytes });
    Ok(())
}

#[test]
fn oversized_request_is_rejected_from_declared_length_before_body_size() -> TestResult {
    // Given
    let bytes = b"tiny";

    // When
    let decision = classify_request_body(bytes, Some(u64::MAX), MaxRequestBodyBytes::new(8)?)?;

    // Then
    assert!(matches!(
        decision,
        ByteDecision::Reject { violation }
            if violation.resource == ByteResource::RequestBody
                && violation.observed == u64::MAX
                && violation.maximum == 8
    ));
    Ok(())
}

#[test]
fn oversized_request_is_rejected_from_actual_opaque_body_size() -> TestResult {
    // Given
    let bytes = [0xff; 9];

    // When
    let decision = classify_request_body(&bytes, None, MaxRequestBodyBytes::new(8)?)?;

    // Then
    assert!(matches!(
        decision,
        ByteDecision::Reject { violation }
            if violation.resource == ByteResource::RequestBody
                && violation.observed == 9
                && violation.maximum == 8
    ));
    Ok(())
}

#[test]
fn malformed_ipc_frame_length_is_a_typed_error() -> TestResult {
    // Given
    let payload = b"frame";

    // When
    let result = classify_ipc_frame(payload, 4, MaxIpcFrameBytes::new(16)?);

    // Then
    assert_eq!(
        result,
        Err(ByteBoundaryError::DeclaredLengthMismatch {
            resource: ByteResource::IpcFrame,
            declared: 4,
            actual: 5,
        })
    );
    Ok(())
}

#[test]
fn oversized_raw_input_bypasses_inspection_with_original_bytes() -> TestResult {
    // Given
    let bytes = [0x00; 9];

    // When
    let decision = classify_raw_bytes(&bytes, MaxRawBytes::new(8)?)?;

    // Then
    assert!(matches!(
        decision,
        ByteDecision::Bypass { bytes: original, violation }
            if original == bytes
                && violation.resource == ByteResource::RawInput
                && violation.observed == 9
                && violation.maximum == 8
    ));
    Ok(())
}

#[test]
fn bomb_shaped_decompressed_length_bypasses_before_allocation() -> TestResult {
    // Given
    let declared = u64::MAX;

    // When
    let decision = classify_decompressed_length(declared, MaxDecompressedBytes::new(1_024)?);

    // Then
    assert!(matches!(
        decision,
        DeclaredLengthDecision::Bypass { violation }
            if violation.observed == u64::MAX && violation.maximum == 1_024
    ));
    Ok(())
}

#[test]
fn overlong_ansi_line_returns_explicit_byte_truncation_metadata() -> TestResult {
    // Given
    let bytes = b"\x1b[31mhostile\x1b[0m";

    // When
    let decision = classify_line_bytes(bytes, MaxLineBytes::new(5)?)?;

    // Then
    assert!(matches!(
        decision,
        ByteDecision::TruncateSafe { bytes: original, metadata }
            if original == bytes
                && metadata.original_bytes == 16
                && metadata.retained_prefix_bytes == 5
                && metadata.omitted_bytes == 11
    ));
    Ok(())
}

#[test]
fn bounded_stream_like_response_terminates_incomplete_without_accumulating_chunks() -> TestResult {
    // Given
    let mut budget = StreamingResponseBudget::new(MaxResponseBodyBytes::new(8)?);

    // When
    let first = budget.observe_chunk(b"1234")?;
    let second = budget.observe_chunk(b"56789")?;

    // Then
    assert_eq!(first, StreamDecision::Continue { accepted_bytes: 4 });
    assert_eq!(
        second,
        StreamDecision::TerminateIncomplete {
            accepted_bytes: 4,
            rejected_chunk_bytes: 5,
            reason: ResponseLimitReason::BodyLimit,
        }
    );
    assert_eq!(budget.accepted_bytes(), 4);
    Ok(())
}

#[test]
fn infinite_response_pattern_reaches_a_finite_incomplete_boundary() -> TestResult {
    // Given
    let mut budget = StreamingResponseBudget::new(MaxResponseBodyBytes::new(8)?);
    let mut terminal = None;

    // When
    for _iteration in 0..9 {
        let decision = budget.observe_chunk(b"x")?;
        if matches!(decision, StreamDecision::TerminateIncomplete { .. }) {
            terminal = Some(decision);
            break;
        }
    }

    // Then
    assert_eq!(
        terminal,
        Some(StreamDecision::TerminateIncomplete {
            accepted_bytes: 8,
            rejected_chunk_bytes: 1,
            reason: ResponseLimitReason::BodyLimit,
        })
    );
    Ok(())
}

#[test]
fn zero_length_response_chunk_preserves_empty_stream_accounting() -> TestResult {
    // Given
    let mut budget = StreamingResponseBudget::new(MaxResponseBodyBytes::new(8)?);

    // When
    let decision = budget.observe_chunk(b"")?;

    // Then
    assert_eq!(decision, StreamDecision::Continue { accepted_bytes: 0 });
    assert_eq!(budget.accepted_bytes(), 0);
    Ok(())
}

#[test]
fn response_stream_rejects_observation_after_termination() -> TestResult {
    // Given
    let mut budget = StreamingResponseBudget::new(MaxResponseBodyBytes::new(8)?);
    let _accepted = budget.observe_chunk(b"12345678")?;
    let _terminal = budget.observe_chunk(b"x")?;

    // When
    let after_terminal = budget.observe_chunk(b"");

    // Then
    assert_eq!(
        after_terminal,
        Err(ByteBoundaryError::StreamAlreadyTerminated)
    );
    assert_eq!(budget.accepted_bytes(), 8);
    Ok(())
}
