use thiserror::Error;

use crate::{
    MaxDecompressedBytes, MaxIpcFrameBytes, MaxLineBytes, MaxRawBytes, MaxRequestBodyBytes,
};

/// The byte-oriented resource being classified.
#[allow(
    clippy::exhaustive_enums,
    reason = "boundary decisions must identify every byte resource explicitly"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteResource {
    /// Opaque raw input.
    RawInput,
    /// Provider request body.
    RequestBody,
    /// Provider response body.
    ResponseBody,
    /// Future decompressed content.
    DecompressedBody,
    /// IPC frame payload.
    IpcFrame,
    /// One opaque line.
    Line,
}

/// Metadata describing a byte count that exceeded a configured limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ByteLimitExceeded {
    /// The resource whose limit was exceeded.
    pub resource: ByteResource,
    /// The observed or declared byte count.
    pub observed: u64,
    /// The configured maximum byte count.
    pub maximum: u64,
}

/// Explicit metadata for deterministic prefix truncation without mutating the source bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TruncationMetadata {
    /// The complete original byte count.
    pub original_bytes: u64,
    /// The number of prefix bytes a caller may retain.
    pub retained_prefix_bytes: u64,
    /// The number of bytes explicitly omitted from the retained prefix.
    pub omitted_bytes: u64,
}

/// A deterministic decision over borrowed opaque bytes.
#[allow(
    clippy::exhaustive_enums,
    reason = "downstream boundaries must handle every byte disposition"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteDecision<'bytes> {
    /// The exact bytes are within the configured boundary.
    WithinLimit {
        /// The original bytes, unchanged.
        bytes: &'bytes [u8],
    },
    /// The input must be rejected before forwarding or decoding.
    Reject {
        /// The typed size violation.
        violation: ByteLimitExceeded,
    },
    /// Inspection is bypassed while the exact original bytes remain available.
    Bypass {
        /// The original bytes, unchanged.
        bytes: &'bytes [u8],
        /// The typed size violation that caused bypass.
        violation: ByteLimitExceeded,
    },
    /// A caller may retain a deterministic prefix using explicit truncation metadata.
    TruncateSafe {
        /// The original bytes, unchanged.
        bytes: &'bytes [u8],
        /// The explicit retained and omitted byte counts.
        metadata: TruncationMetadata,
    },
}

/// A decision based only on an untrusted declared length.
#[allow(
    clippy::exhaustive_enums,
    reason = "declared lengths are either safe to inspect or require bypass"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclaredLengthDecision {
    /// The declared length is within the configured boundary.
    WithinLimit,
    /// Work must be bypassed before allocating from the declared length.
    Bypass {
        /// The typed declared-length violation.
        violation: ByteLimitExceeded,
    },
}

/// Failure to classify byte-oriented boundary metadata.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ByteBoundaryError {
    /// A declared frame or body length did not equal the bytes already received.
    #[error("{resource:?} declared {declared} bytes but contained {actual}")]
    DeclaredLengthMismatch {
        /// The malformed resource.
        resource: ByteResource,
        /// The untrusted declared byte count.
        declared: u64,
        /// The byte count actually present.
        actual: u64,
    },
    /// A platform byte count could not be represented by the protocol counter.
    #[error("{resource:?} byte count cannot be represented as u64")]
    LengthUnrepresentable {
        /// The resource whose platform length could not be represented.
        resource: ByteResource,
    },
    /// A response stream was observed after it had already terminated.
    #[error("response stream budget was already terminated")]
    StreamAlreadyTerminated,
}

/// Classifies opaque raw bytes for bounded inspection.
///
/// # Errors
/// Returns [`ByteBoundaryError`] if the platform byte count cannot fit the protocol counter.
pub fn classify_raw_bytes(
    bytes: &[u8],
    maximum: MaxRawBytes,
) -> Result<ByteDecision<'_>, ByteBoundaryError> {
    let observed = measured_len(bytes, ByteResource::RawInput)?;
    if observed > maximum.get() {
        Ok(ByteDecision::Bypass {
            bytes,
            violation: violation(ByteResource::RawInput, observed, maximum.get()),
        })
    } else {
        Ok(ByteDecision::WithinLimit { bytes })
    }
}

/// Classifies a request body, checking its claimed length before its received byte count.
///
/// # Errors
/// Returns [`ByteBoundaryError`] for malformed declared lengths or unrepresentable byte counts.
pub fn classify_request_body(
    bytes: &[u8],
    declared_length: Option<u64>,
    maximum: MaxRequestBodyBytes,
) -> Result<ByteDecision<'_>, ByteBoundaryError> {
    classify_framed_bytes(
        bytes,
        declared_length,
        FramedBoundary {
            maximum: maximum.get(),
            resource: ByteResource::RequestBody,
        },
    )
}

/// Classifies an IPC frame without allocating from its untrusted declared length.
///
/// # Errors
/// Returns [`ByteBoundaryError`] for malformed declared lengths or unrepresentable byte counts.
pub fn classify_ipc_frame(
    bytes: &[u8],
    declared_length: u64,
    maximum: MaxIpcFrameBytes,
) -> Result<ByteDecision<'_>, ByteBoundaryError> {
    classify_framed_bytes(
        bytes,
        Some(declared_length),
        FramedBoundary {
            maximum: maximum.get(),
            resource: ByteResource::IpcFrame,
        },
    )
}

/// Classifies a future decompressed length without performing decompression or allocation.
#[must_use]
pub const fn classify_decompressed_length(
    declared_length: u64,
    maximum: MaxDecompressedBytes,
) -> DeclaredLengthDecision {
    if declared_length > maximum.get() {
        DeclaredLengthDecision::Bypass {
            violation: violation(
                ByteResource::DecompressedBody,
                declared_length,
                maximum.get(),
            ),
        }
    } else {
        DeclaredLengthDecision::WithinLimit
    }
}

/// Classifies one opaque line and reports safe prefix truncation metadata when overlong.
///
/// # Errors
/// Returns [`ByteBoundaryError`] if the platform byte count cannot fit the protocol counter.
pub fn classify_line_bytes(
    bytes: &[u8],
    maximum: MaxLineBytes,
) -> Result<ByteDecision<'_>, ByteBoundaryError> {
    let observed = measured_len(bytes, ByteResource::Line)?;
    if observed > maximum.get() {
        let omitted_bytes = observed.checked_sub(maximum.get()).ok_or(
            ByteBoundaryError::LengthUnrepresentable {
                resource: ByteResource::Line,
            },
        )?;
        Ok(ByteDecision::TruncateSafe {
            bytes,
            metadata: TruncationMetadata {
                original_bytes: observed,
                retained_prefix_bytes: maximum.get(),
                omitted_bytes,
            },
        })
    } else {
        Ok(ByteDecision::WithinLimit { bytes })
    }
}

#[derive(Clone, Copy)]
struct FramedBoundary {
    maximum: u64,
    resource: ByteResource,
}

fn classify_framed_bytes(
    bytes: &[u8],
    declared_length: Option<u64>,
    boundary: FramedBoundary,
) -> Result<ByteDecision<'_>, ByteBoundaryError> {
    if let Some(declared) = declared_length
        && declared > boundary.maximum
    {
        return Ok(ByteDecision::Reject {
            violation: violation(boundary.resource, declared, boundary.maximum),
        });
    }
    let actual = measured_len(bytes, boundary.resource)?;
    if actual > boundary.maximum {
        return Ok(ByteDecision::Reject {
            violation: violation(boundary.resource, actual, boundary.maximum),
        });
    }
    if let Some(declared) = declared_length
        && declared != actual
    {
        return Err(ByteBoundaryError::DeclaredLengthMismatch {
            resource: boundary.resource,
            declared,
            actual,
        });
    }
    Ok(ByteDecision::WithinLimit { bytes })
}

fn measured_len(bytes: &[u8], resource: ByteResource) -> Result<u64, ByteBoundaryError> {
    u64::try_from(bytes.len())
        .map_err(|_error| ByteBoundaryError::LengthUnrepresentable { resource })
}

const fn violation(resource: ByteResource, observed: u64, maximum: u64) -> ByteLimitExceeded {
    ByteLimitExceeded {
        resource,
        observed,
        maximum,
    }
}
