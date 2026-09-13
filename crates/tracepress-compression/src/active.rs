//! Explicitly opt-in request rewriting primitives for the Phase 4.2 experiment.
//!
//! This module is deliberately independent from the proxy. It receives spans produced by the
//! context analyser and returns a transient rewritten body. The caller must keep the returned
//! bytes on the experiment path only; no default Tracepress forwarding path invokes this module.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{CompressionLimits, JsonMinify, ShadowCompressor, TransformError};

/// One JSON value span in the request being evaluated.
///
/// `encoded_string` is true for a Responses `function_call_output.output` string: the compressor
/// sees the decoded string bytes, while the replacement is encoded as one JSON string again. For
/// object/array values the span is replaced directly with the minified JSON document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveJsonSpan {
    /// First byte of the JSON value, including quotes for an encoded string.
    pub start: u64,
    /// One past the last byte of the JSON value.
    pub end: u64,
    /// Whether this span is a JSON string carrying a JSON document.
    pub encoded_string: bool,
}

impl ActiveJsonSpan {
    /// Creates a validated non-empty span.
    #[must_use]
    pub const fn new(start: u64, end: u64, encoded_string: bool) -> Option<Self> {
        if start < end {
            Some(Self {
                start,
                end,
                encoded_string,
            })
        } else {
            None
        }
    }
}

/// Terminal state of one active rewrite attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum ActiveRewriteStatus {
    /// At least one selected span was replaced by a smaller lossless JSON document.
    Rewritten,
    /// The selected spans were valid but no encoded replacement was smaller.
    NoImprovement,
    /// No valid target span was supplied, or all selected spans were outside this compressor's
    /// narrow contract.
    NotApplicable,
    /// A configured work, output, or memory bound stopped the attempt.
    ResourceLimit,
    /// A selected span was not a valid JSON document/string value.
    InvalidInput,
    /// The compressor could not recover one selected span byte-for-byte.
    RecoveryFailed,
    /// The same input produced different candidate or recovery bytes.
    InternalError,
}

/// Metadata for one explicitly enabled active rewrite attempt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ActiveRewriteMetrics {
    /// Stable active candidate identifier.
    pub compressor_id: String,
    /// Candidate implementation version.
    pub compressor_version: u32,
    /// Terminal attempt state.
    pub status: ActiveRewriteStatus,
    /// Original full request bytes.
    pub input_bytes: u64,
    /// Rewritten full request bytes, when a rewrite was accepted.
    pub output_bytes: Option<u64>,
    /// Positive byte reduction, when a rewrite was accepted.
    pub bytes_delta: Option<i64>,
    /// Number of target spans replaced.
    pub rewrites: u32,
    /// Whether every transformed span recovered its exact decoded input.
    pub recovery_verified: bool,
    /// Whether repeated transformation produced identical bytes.
    pub deterministic: bool,
    /// SHA-256 over the original request body.
    pub original_fingerprint: [u8; 32],
    /// SHA-256 over the rewritten request body, when a rewrite was accepted.
    pub rewritten_fingerprint: Option<[u8; 32]>,
    /// First request-relative byte changed by the accepted rewrite.
    pub first_modified_offset: Option<u64>,
    /// Request-relative common prefix length.
    pub preserved_prefix_bytes: Option<u64>,
}

/// Transient result of an active rewrite. The body is intentionally omitted from [`Debug`] and
/// has no serialization implementation, so it cannot cross the metadata persistence boundary.
pub struct ActiveRewrite {
    metrics: ActiveRewriteMetrics,
    body: Option<Box<[u8]>>,
}

impl fmt::Debug for ActiveRewrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActiveRewrite")
            .field("metrics", &self.metrics)
            .field("body_bytes", &self.body.as_ref().map(|body| body.len()))
            .finish()
    }
}

impl ActiveRewrite {
    /// Returns metadata safe to persist or publish to an experiment observer.
    #[must_use]
    pub const fn metrics(&self) -> &ActiveRewriteMetrics {
        &self.metrics
    }

    /// Borrows the transient rewritten request body, if a rewrite was accepted.
    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }

    /// Consumes the result and returns the transient body to the explicit experiment caller.
    #[must_use]
    pub fn into_body(self) -> Option<Box<[u8]>> {
        self.body
    }
}

/// Rewrites selected `ToolResult` JSON spans with the lossless `json.minify` candidate.
///
/// This is not called by normal forwarding. It is an adapter for an explicitly enabled Phase 4.2
/// arm after Phase 3 has supplied metadata-only spans. All spans are validated before any output
/// is accepted; malformed, overlapping, non-deterministic, or non-recoverable attempts return the
/// original-body result with no replacement bytes.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "the active adapter keeps validation, recovery, and fail-open gates in one bounded state machine"
)]
pub fn rewrite_json_minify(
    input: &[u8],
    spans: &[ActiveJsonSpan],
    limits: &CompressionLimits,
) -> ActiveRewrite {
    let original_fingerprint = digest(input);
    let base = |status| ActiveRewrite {
        metrics: ActiveRewriteMetrics {
            compressor_id: "json.minify".to_owned(),
            compressor_version: 1,
            status,
            input_bytes: u64::try_from(input.len()).unwrap_or(u64::MAX),
            output_bytes: None,
            bytes_delta: None,
            rewrites: 0,
            recovery_verified: false,
            deterministic: false,
            original_fingerprint,
            rewritten_fingerprint: None,
            first_modified_offset: None,
            preserved_prefix_bytes: None,
        },
        body: None,
    };

    if spans.is_empty() {
        return base(ActiveRewriteStatus::NotApplicable);
    }
    if u64::try_from(input.len()).unwrap_or(u64::MAX) > limits.max_shadow_memory_bytes {
        return base(ActiveRewriteStatus::ResourceLimit);
    }

    let mut ordered = spans.to_vec();
    ordered.sort_by_key(|span| (span.start, span.end));
    let mut replacements = Vec::with_capacity(ordered.len());
    let mut previous_end = 0_u64;
    let mut work = 0_u64;
    let mut terminal = None;
    let mut evaluated_spans = 0_u32;
    let mut all_deterministic = true;
    let mut all_recovery_verified = true;
    let compressor = JsonMinify;

    for span in ordered {
        if span.start < previous_end {
            return base(ActiveRewriteStatus::InvalidInput);
        }
        let Ok(start) = usize::try_from(span.start) else {
            return base(ActiveRewriteStatus::InvalidInput);
        };
        let Ok(end) = usize::try_from(span.end) else {
            return base(ActiveRewriteStatus::InvalidInput);
        };
        let Some(raw) = input.get(start..end) else {
            return base(ActiveRewriteStatus::InvalidInput);
        };
        previous_end = span.end;
        let content = if span.encoded_string {
            let Ok(decoded) = serde_json::from_slice::<String>(raw) else {
                terminal = Some(ActiveRewriteStatus::InvalidInput);
                continue;
            };
            decoded.into_bytes()
        } else {
            raw.to_vec()
        };
        work = work.saturating_add(u64::try_from(content.len()).unwrap_or(u64::MAX));
        if work > limits.max_shadow_work_units
            || u64::try_from(content.len()).unwrap_or(u64::MAX) > limits.max_candidate_input_bytes
        {
            terminal = Some(ActiveRewriteStatus::ResourceLimit);
            continue;
        }

        let first = match compressor.transform(&content, limits) {
            Ok(value) => value,
            Err(error) => {
                terminal = Some(status_for_transform(error));
                continue;
            }
        };
        let second = match compressor.transform(&content, limits) {
            Ok(value) => value,
            Err(error) => {
                terminal = Some(status_for_transform(error));
                continue;
            }
        };
        if first.candidate_bytes() != second.candidate_bytes()
            || first.recovery_bytes() != second.recovery_bytes()
        {
            all_deterministic = false;
            terminal = Some(ActiveRewriteStatus::InternalError);
            continue;
        }
        evaluated_spans = evaluated_spans.saturating_add(1);
        let Ok(recovered) =
            compressor.recover(first.candidate_bytes(), first.recovery_bytes(), limits)
        else {
            all_recovery_verified = false;
            terminal = Some(ActiveRewriteStatus::RecoveryFailed);
            continue;
        };
        if digest(&recovered) != digest(&content) {
            all_recovery_verified = false;
            terminal = Some(ActiveRewriteStatus::RecoveryFailed);
            continue;
        }
        let replacement = if span.encoded_string {
            let Ok(candidate_text) = std::str::from_utf8(first.candidate_bytes()) else {
                terminal = Some(ActiveRewriteStatus::InvalidInput);
                continue;
            };
            let Ok(encoded) = serde_json::to_vec(candidate_text) else {
                terminal = Some(ActiveRewriteStatus::InternalError);
                continue;
            };
            encoded
        } else {
            first.candidate_bytes().to_vec()
        };
        if u64::try_from(replacement.len()).unwrap_or(u64::MAX) > limits.max_candidate_output_bytes
        {
            terminal = Some(ActiveRewriteStatus::ResourceLimit);
            continue;
        }
        // For encoded strings the comparison includes the enclosing quotes and escaping.
        if replacement.len() < raw.len() {
            replacements.push((start, end, replacement));
        } else {
            let _ = terminal.get_or_insert(ActiveRewriteStatus::NoImprovement);
        }
    }

    if replacements.is_empty() {
        let status = terminal.unwrap_or(ActiveRewriteStatus::NoImprovement);
        let mut result = base(status);
        result.metrics.deterministic = evaluated_spans > 0 && all_deterministic;
        result.metrics.recovery_verified = evaluated_spans > 0 && all_recovery_verified;
        return result;
    }
    if terminal.is_some_and(|status| {
        matches!(
            status,
            ActiveRewriteStatus::ResourceLimit
                | ActiveRewriteStatus::InvalidInput
                | ActiveRewriteStatus::RecoveryFailed
                | ActiveRewriteStatus::InternalError
        )
    }) {
        return base(terminal.unwrap_or(ActiveRewriteStatus::InternalError));
    }

    let mut body = Vec::with_capacity(input.len());
    let mut cursor = 0_usize;
    for (start, end, replacement) in &replacements {
        let Some(prefix) = input.get(cursor..*start) else {
            return base(ActiveRewriteStatus::InternalError);
        };
        body.extend_from_slice(prefix);
        body.extend_from_slice(replacement);
        cursor = *end;
    }
    let Some(suffix) = input.get(cursor..) else {
        return base(ActiveRewriteStatus::InternalError);
    };
    body.extend_from_slice(suffix);
    if u64::try_from(body.len()).unwrap_or(u64::MAX) > limits.max_shadow_memory_bytes {
        return base(ActiveRewriteStatus::ResourceLimit);
    }
    let prefix = common_prefix(input, &body);
    let input_bytes = u64::try_from(input.len()).unwrap_or(u64::MAX);
    let output_bytes = u64::try_from(body.len()).unwrap_or(u64::MAX);
    ActiveRewrite {
        metrics: ActiveRewriteMetrics {
            compressor_id: "json.minify".to_owned(),
            compressor_version: 1,
            status: ActiveRewriteStatus::Rewritten,
            input_bytes,
            output_bytes: Some(output_bytes),
            bytes_delta: signed_delta(input_bytes, output_bytes),
            rewrites: u32::try_from(replacements.len()).unwrap_or(u32::MAX),
            recovery_verified: true,
            deterministic: true,
            original_fingerprint,
            rewritten_fingerprint: Some(digest(&body)),
            first_modified_offset: (prefix < input.len())
                .then_some(u64::try_from(prefix).unwrap_or(u64::MAX)),
            preserved_prefix_bytes: Some(u64::try_from(prefix).unwrap_or(u64::MAX)),
        },
        body: Some(body.into_boxed_slice()),
    }
}

const fn status_for_transform(error: TransformError) -> ActiveRewriteStatus {
    match error {
        TransformError::NotApplicable => ActiveRewriteStatus::NotApplicable,
        TransformError::ResourceLimit => ActiveRewriteStatus::ResourceLimit,
        TransformError::InvalidInput => ActiveRewriteStatus::InvalidInput,
        TransformError::Internal => ActiveRewriteStatus::InternalError,
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    left.iter().zip(right).take_while(|(a, b)| a == b).count()
}

fn signed_delta(input: u64, output: u64) -> Option<i64> {
    if output >= input {
        i64::try_from(output.saturating_sub(input))
            .ok()
            .map(i64::saturating_neg)
    } else {
        i64::try_from(input.saturating_sub(output)).ok()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "local active adapter fixtures"
)]
mod tests {
    use super::*;

    fn limits() -> CompressionLimits {
        CompressionLimits::default()
    }

    fn output_span(request: &[u8]) -> ActiveJsonSpan {
        let marker = b"\"output\":\"";
        let marker_start = request
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("output marker");
        let start = marker_start + marker.len() - 1;
        let mut escaped = false;
        for (offset, byte) in request[start + 1..].iter().copied().enumerate() {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return ActiveJsonSpan::new(
                    u64::try_from(start).expect("span start"),
                    u64::try_from(start + 2 + offset).expect("span end"),
                    true,
                )
                .expect("non-empty output span");
            }
        }
        panic!("unterminated output string");
    }

    #[test]
    fn rewrites_only_selected_encoded_tool_result_and_recovers_exact_value() {
        let request = br#"{"model":"test","input":[{"type":"function_call_output","output":"{ \"name\": \"a\", \"size\": 10 }"}],"keep":"exact"}"#;
        let span = output_span(request);
        let result = rewrite_json_minify(request, &[span], &limits());
        assert_eq!(result.metrics().status, ActiveRewriteStatus::Rewritten);
        assert!(result.metrics().recovery_verified);
        assert!(result.metrics().deterministic);
        assert!(result.metrics().output_bytes.unwrap_or_default() < request.len() as u64);
        let body = result.body().expect("rewritten body");
        let value: serde_json::Value = serde_json::from_slice(body).expect("rewritten JSON");
        assert_eq!(
            value["input"][0]["output"],
            serde_json::Value::String(r#"{"name":"a","size":10}"#.to_owned())
        );
    }

    #[test]
    fn disabled_by_empty_span_set_without_body() {
        let result = rewrite_json_minify(br#"{"input":[]}"#, &[], &limits());
        assert_eq!(result.metrics().status, ActiveRewriteStatus::NotApplicable);
        assert!(result.body().is_none());
    }

    #[test]
    fn duplicate_keys_are_not_rewritten() {
        let request = br#"{"output":"{ \"x\":1, \"x\":2 }"}"#;
        let span = output_span(request);
        let result = rewrite_json_minify(request, &[span], &limits());
        assert_eq!(result.metrics().status, ActiveRewriteStatus::NotApplicable);
        assert!(result.body().is_none());
    }

    #[test]
    fn debug_does_not_include_body_content() {
        let request = br#"{"output":"{ \"secret\": 1 }"}"#;
        let span = output_span(request);
        let result = rewrite_json_minify(request, &[span], &limits());
        let debug = format!("{result:?}");
        assert!(!debug.contains("secret"));
        assert!(debug.contains("body_bytes"));
    }
}
