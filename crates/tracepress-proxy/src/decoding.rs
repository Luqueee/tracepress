//! Analysis-only decoding for provider request bodies.

#![allow(
    clippy::redundant_pub_crate,
    clippy::struct_field_names,
    clippy::too_many_arguments,
    reason = "the decoder contract names explicit independent resource dimensions"
)]

use std::fmt;
use std::io::{Cursor, Read as _};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use tracepress_provider::{AnalysisDecodeStatus, ContentEncoding};

const DECODER_VERSION: u32 = 1;
const DECODE_BUFFER_BYTES: usize = 8192;

/// Exact bytes accepted by the forwarding path.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct WireBody(Bytes);

impl WireBody {
    pub(crate) const fn new(bytes: Bytes) -> Self {
        Self(bytes)
    }

    pub(crate) fn as_ref(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn clone_bytes(&self) -> Bytes {
        self.0.clone()
    }

    pub(crate) const fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for WireBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WireBody")
            .field("len", &self.len())
            .finish()
    }
}

/// Bytes created only for the shadow analyzer.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct AnalysisBody(Bytes);

impl AnalysisBody {
    pub(crate) const fn new(bytes: Bytes) -> Self {
        Self(bytes)
    }

    pub(crate) fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for AnalysisBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnalysisBody")
            .field("len", &self.0.len())
            .finish()
    }
}

/// Bounds applied to the analysis-only decoder.
#[derive(Clone, Debug)]
pub(crate) struct DecodeLimits {
    pub(crate) max_compressed_bytes: u64,
    pub(crate) max_decompressed_bytes: u64,
    pub(crate) max_expansion_ratio: Option<f64>,
    pub(crate) max_decode_time: Duration,
}

/// Result and bounded metadata from one analysis decode attempt.
#[derive(Debug)]
pub(crate) struct DecodeResult {
    pub(crate) status: AnalysisDecodeStatus,
    pub(crate) body: Option<AnalysisBody>,
    pub(crate) wire_bytes: u64,
    pub(crate) decoded_bytes: u64,
    pub(crate) decode_duration_us: u64,
    pub(crate) decoder_version: u32,
}

/// Decoder boundary used by shadow context analysis.
pub(crate) trait AnalysisDecoder {
    fn decode(
        &self,
        wire: &WireBody,
        encoding: ContentEncoding,
        limits: &DecodeLimits,
    ) -> DecodeResult;
}

/// Streaming zstd decoder with an absolute output and time budget.
pub(crate) struct BoundedAnalysisDecoder;

impl AnalysisDecoder for BoundedAnalysisDecoder {
    fn decode(
        &self,
        wire: &WireBody,
        encoding: ContentEncoding,
        limits: &DecodeLimits,
    ) -> DecodeResult {
        let started = Instant::now();
        let wire_bytes = u64::try_from(wire.len()).unwrap_or(u64::MAX);
        let finish = |status, body, decoded_bytes| DecodeResult {
            status,
            body,
            wire_bytes,
            decoded_bytes,
            decode_duration_us: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            decoder_version: DECODER_VERSION,
        };

        if wire_bytes > limits.max_compressed_bytes {
            return finish(AnalysisDecodeStatus::ResourceLimit, None, 0);
        }
        match encoding {
            ContentEncoding::Identity => finish(
                AnalysisDecodeStatus::Identity,
                Some(AnalysisBody::new(wire.clone_bytes())),
                wire_bytes,
            ),
            ContentEncoding::Unsupported => {
                finish(AnalysisDecodeStatus::UnsupportedEncoding, None, 0)
            }
            ContentEncoding::Zstd => decode_zstd(wire, limits, started, finish),
            _ => finish(AnalysisDecodeStatus::UnsupportedEncoding, None, 0),
        }
    }
}

fn decode_zstd(
    wire: &WireBody,
    limits: &DecodeLimits,
    started: Instant,
    finish: impl Fn(AnalysisDecodeStatus, Option<AnalysisBody>, u64) -> DecodeResult,
) -> DecodeResult {
    let mut decoder = match zstd::stream::read::Decoder::new(Cursor::new(wire.as_ref())) {
        Ok(decoder) => decoder,
        Err(_error) => return finish(AnalysisDecodeStatus::CorruptPayload, None, 0),
    };
    let mut output = Vec::new();
    let mut buffer = [0_u8; DECODE_BUFFER_BYTES];
    let mut decoded_bytes = 0_u64;
    loop {
        if started.elapsed() > limits.max_decode_time {
            return finish(AnalysisDecodeStatus::Timeout, None, decoded_bytes);
        }
        let remaining = limits.max_decompressed_bytes.saturating_sub(decoded_bytes);
        let read_capacity = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        if read_capacity == 0 {
            return finish(AnalysisDecodeStatus::ResourceLimit, None, decoded_bytes);
        }
        let Some(read_buffer) = buffer.get_mut(..read_capacity) else {
            return finish(AnalysisDecodeStatus::CorruptPayload, None, decoded_bytes);
        };
        let read = match decoder.read(read_buffer) {
            Ok(read) => read,
            Err(_error) => {
                return finish(AnalysisDecodeStatus::CorruptPayload, None, decoded_bytes);
            }
        };
        if read == 0 {
            break;
        }
        let read_bytes = u64::try_from(read).unwrap_or(u64::MAX);
        decoded_bytes = decoded_bytes.saturating_add(read_bytes);
        if let Some(ratio) = limits.max_expansion_ratio {
            #[allow(
                clippy::cast_precision_loss,
                reason = "the optional ratio is explicitly a floating-point operator setting"
            )]
            let exceeds = wire.len() > 0
                && ratio.is_finite()
                && ratio > 0.0
                && (decoded_bytes as f64 / wire.len() as f64) > ratio;
            if exceeds {
                return finish(AnalysisDecodeStatus::ResourceLimit, None, decoded_bytes);
            }
        }
        let Some(chunk) = buffer.get(..read) else {
            return finish(AnalysisDecodeStatus::CorruptPayload, None, decoded_bytes);
        };
        output.extend_from_slice(chunk);
    }
    finish(
        AnalysisDecodeStatus::Decoded,
        Some(AnalysisBody::new(Bytes::from(output))),
        decoded_bytes,
    )
}

/// Parses one Content-Encoding value without guessing chained encodings.
pub(crate) fn parse_content_encoding(value: Option<&str>) -> ContentEncoding {
    match value.map(str::trim) {
        None => ContentEncoding::Identity,
        Some(value) if value.eq_ignore_ascii_case("identity") => ContentEncoding::Identity,
        Some(value) if value.eq_ignore_ascii_case("zstd") => ContentEncoding::Zstd,
        Some(_) => ContentEncoding::Unsupported,
    }
}

/// Parses a header value, treating invalid header text as unsupported rather than identity.
pub(crate) fn parse_content_encoding_header(
    value: Option<&axum::http::HeaderValue>,
) -> ContentEncoding {
    let Some(value) = value else {
        return ContentEncoding::Identity;
    };
    match value.to_str() {
        Ok(value) => parse_content_encoding(Some(value)),
        Err(_error) => ContentEncoding::Unsupported,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "decoder unit fixtures are local"
)]
mod tests {
    use axum::body::Bytes;
    use tracepress_provider::{AnalysisDecodeStatus, ContentEncoding};

    use super::*;

    fn limits() -> DecodeLimits {
        DecodeLimits {
            max_compressed_bytes: 1024,
            max_decompressed_bytes: 4096,
            max_expansion_ratio: None,
            max_decode_time: std::time::Duration::from_secs(1),
        }
    }

    #[test]
    fn identity_uses_the_original_body_as_analysis_body() {
        let wire = WireBody::new(Bytes::from_static(br#"{"input":"ok"}"#));
        let result = BoundedAnalysisDecoder.decode(&wire, ContentEncoding::Identity, &limits());
        assert_eq!(result.status, AnalysisDecodeStatus::Identity);
        assert_eq!(result.wire_bytes, wire.len() as u64);
        assert_eq!(result.decoded_bytes, wire.len() as u64);
        assert_eq!(
            result.body.as_ref().map(AnalysisBody::as_ref),
            Some(wire.as_ref())
        );
    }

    #[test]
    fn zstd_is_decoded_without_changing_the_wire_body() {
        let json = br#"{"model":"gpt-5","input":[{"role":"user","content":"hello"}]}"#;
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(json), 1).unwrap();
        let wire = WireBody::new(Bytes::from(compressed));
        let result = BoundedAnalysisDecoder.decode(&wire, ContentEncoding::Zstd, &limits());
        assert_eq!(result.status, AnalysisDecodeStatus::Decoded);
        assert_eq!(result.decoded_bytes, json.len() as u64);
        assert_eq!(result.body.unwrap().as_ref(), json);
        assert_ne!(wire.as_ref(), json);
    }

    #[test]
    fn unsupported_and_corrupt_payloads_fail_open_without_analysis_body() {
        let wire = WireBody::new(Bytes::from_static(b"not-json"));
        let unsupported =
            BoundedAnalysisDecoder.decode(&wire, ContentEncoding::Unsupported, &limits());
        assert_eq!(
            unsupported.status,
            AnalysisDecodeStatus::UnsupportedEncoding
        );
        assert!(unsupported.body.is_none());

        let corrupt = BoundedAnalysisDecoder.decode(&wire, ContentEncoding::Zstd, &limits());
        assert_eq!(corrupt.status, AnalysisDecodeStatus::CorruptPayload);
        assert!(corrupt.body.is_none());
    }

    #[test]
    fn decompression_is_stopped_at_the_absolute_output_limit() {
        let json = vec![b'x'; 8192];
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(json), 1).unwrap();
        let wire = WireBody::new(Bytes::from(compressed));
        let mut bounded = limits();
        bounded.max_decompressed_bytes = 128;
        let result = BoundedAnalysisDecoder.decode(&wire, ContentEncoding::Zstd, &bounded);
        assert_eq!(result.status, AnalysisDecodeStatus::ResourceLimit);
        assert!(result.body.is_none());
        assert!(result.decoded_bytes <= bounded.max_decompressed_bytes);
    }

    #[test]
    fn content_encoding_parsing_is_case_insensitive_but_does_not_guess_chains() {
        assert_eq!(parse_content_encoding(None), ContentEncoding::Identity);
        assert_eq!(parse_content_encoding(Some("ZsTd")), ContentEncoding::Zstd);
        assert_eq!(
            parse_content_encoding(Some("gzip, zstd")),
            ContentEncoding::Unsupported
        );
        assert_eq!(
            parse_content_encoding(Some("")),
            ContentEncoding::Unsupported
        );
    }
}
