//! Exact-limit and one-over-limit table for byte-oriented resource axes.

use tracepress_core::{
    ByteDecision, ByteResource, DeclaredLengthDecision, MaxDecompressedBytes, MaxIpcFrameBytes,
    MaxLineBytes, MaxRawBytes, MaxRequestBodyBytes, classify_decompressed_length,
    classify_ipc_frame, classify_line_bytes, classify_raw_bytes, classify_request_body,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug)]
enum ByteBoundaryKind {
    Raw,
    Request,
    IpcFrame,
    Line,
    DecompressedDeclaration,
}

const BYTE_BOUNDARIES: [ByteBoundaryKind; 5] = [
    ByteBoundaryKind::Raw,
    ByteBoundaryKind::Request,
    ByteBoundaryKind::IpcFrame,
    ByteBoundaryKind::Line,
    ByteBoundaryKind::DecompressedDeclaration,
];

#[test]
fn byte_boundaries_accept_exact_and_classify_one_over() -> TestResult {
    // Given
    let exact = [0_u8; 8];
    let one_over = [0_u8; 9];

    // When / Then
    for boundary in BYTE_BOUNDARIES {
        match boundary {
            ByteBoundaryKind::Raw => {
                let maximum = MaxRawBytes::new(8)?;
                assert_eq!(
                    classify_raw_bytes(&exact, maximum)?,
                    ByteDecision::WithinLimit { bytes: &exact }
                );
                let over = classify_raw_bytes(&one_over, maximum)?;
                assert!(matches!(
                    over,
                    ByteDecision::Bypass { bytes, violation }
                        if bytes == one_over
                            && violation.resource == ByteResource::RawInput
                            && violation.observed == 9
                            && violation.maximum == 8
                ));
            }
            ByteBoundaryKind::Request => {
                let maximum = MaxRequestBodyBytes::new(8)?;
                assert_eq!(
                    classify_request_body(&exact, Some(8), maximum)?,
                    ByteDecision::WithinLimit { bytes: &exact }
                );
                let over = classify_request_body(&one_over, Some(9), maximum)?;
                assert!(matches!(
                    over,
                    ByteDecision::Reject { violation }
                        if violation.resource == ByteResource::RequestBody
                            && violation.observed == 9
                            && violation.maximum == 8
                ));
            }
            ByteBoundaryKind::IpcFrame => {
                let maximum = MaxIpcFrameBytes::new(8)?;
                assert_eq!(
                    classify_ipc_frame(&exact, 8, maximum)?,
                    ByteDecision::WithinLimit { bytes: &exact }
                );
                let over = classify_ipc_frame(&one_over, 9, maximum)?;
                assert!(matches!(
                    over,
                    ByteDecision::Reject { violation }
                        if violation.resource == ByteResource::IpcFrame
                            && violation.observed == 9
                            && violation.maximum == 8
                ));
            }
            ByteBoundaryKind::Line => {
                let maximum = MaxLineBytes::new(8)?;
                assert_eq!(
                    classify_line_bytes(&exact, maximum)?,
                    ByteDecision::WithinLimit { bytes: &exact }
                );
                let over = classify_line_bytes(&one_over, maximum)?;
                assert!(matches!(
                    over,
                    ByteDecision::TruncateSafe { bytes, metadata }
                        if bytes == one_over
                            && metadata.original_bytes == 9
                            && metadata.retained_prefix_bytes == 8
                            && metadata.omitted_bytes == 1
                ));
            }
            ByteBoundaryKind::DecompressedDeclaration => {
                let maximum = MaxDecompressedBytes::new(8)?;
                assert_eq!(
                    classify_decompressed_length(8, maximum),
                    DeclaredLengthDecision::WithinLimit
                );
                let over = classify_decompressed_length(9, maximum);
                assert!(matches!(
                    over,
                    DeclaredLengthDecision::Bypass { violation }
                        if violation.resource == ByteResource::DecompressedBody
                            && violation.observed == 9
                            && violation.maximum == 8
                ));
            }
        }
    }
    Ok(())
}
