//! Explicitly opt-in request rewriting primitives for the Phase 4.2 experiment.
//!
//! This module is deliberately independent from the proxy. It receives spans produced by the
//! context analyser and returns a transient rewritten body. The caller must keep the returned
//! bytes on the experiment path only; no default Tracepress forwarding path invokes this module.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    CompressionLimits, JsonMinify, ReductionStatus, SearchResultReducer, ShadowCompressor,
    ToolResultReducer, TransformError,
};

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

/// One bounded `ToolResult` span in a Responses request.
///
/// `encoded_string` is true for a Responses `function_call_output.output` string: the reducer
/// sees decoded bytes while the replacement is encoded as one JSON string again. The bytes may
/// be raw Search output or a JSON envelope containing exactly one Search-output string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveTextSpan {
    /// First byte of the JSON value, including quotes for an encoded string.
    pub start: u64,
    /// One past the last byte of the JSON value.
    pub end: u64,
    /// Whether this span is a JSON string carrying plain text.
    pub encoded_string: bool,
}

impl ActiveTextSpan {
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
    /// Bytes of the decoded representation evaluated by the candidate.
    pub input_bytes: u64,
    /// Bytes of the decoded candidate representation, when a rewrite was accepted.
    pub output_bytes: Option<u64>,
    /// Positive decoded-representation reduction, when a rewrite was accepted.
    pub bytes_delta: Option<i64>,
    /// Number of target spans replaced.
    pub rewrites: u32,
    /// Number of eligible spans whose candidate was evaluated, including non-improvements.
    pub evaluated_spans: u32,
    /// Total decoded bytes in evaluated source spans.
    pub evaluated_input_bytes: u64,
    /// Total encoded candidate bytes for evaluated spans.
    pub evaluated_candidate_bytes: u64,
    /// Whether every transformed span recovered its exact decoded input.
    pub recovery_verified: bool,
    /// Whether repeated transformation produced identical bytes.
    pub deterministic: bool,
    /// SHA-256 over the decoded representation evaluated by the candidate.
    pub original_fingerprint: [u8; 32],
    /// SHA-256 over the rewritten decoded representation, when a rewrite was accepted.
    pub rewritten_fingerprint: Option<[u8; 32]>,
    /// First decoded-request byte changed by the accepted rewrite.
    pub first_modified_offset: Option<u64>,
    /// Decoded-request common prefix length.
    pub preserved_prefix_bytes: Option<u64>,
    /// Original bytes on the transport wire, when a decoded rewrite was reframed.
    pub wire_input_bytes: Option<u64>,
    /// Re-encoded bytes on the transport wire, when a decoded rewrite was reframed.
    pub wire_output_bytes: Option<u64>,
    /// Signed wire-size delta; a negative value means wire expansion.
    pub wire_bytes_delta: Option<i64>,
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

    /// Reframes a successful decoded rewrite onto the bytes that will actually cross the wire.
    ///
    /// This is used by transports such as zstd where the compressor operates on decoded JSON but
    /// the provider receives a re-encoded body. The never-worse guard belongs to the decoded
    /// candidate, while the wire size records transport expansion or reduction explicitly.
    #[must_use]
    pub fn reframe(self, original_wire: &[u8], rewritten_wire: Box<[u8]>) -> Self {
        let Self {
            mut metrics,
            body: _body,
        } = self;
        if !matches!(metrics.status, ActiveRewriteStatus::Rewritten) {
            return Self {
                metrics,
                body: None,
            };
        }
        let input_bytes = u64::try_from(original_wire.len()).unwrap_or(u64::MAX);
        let output_bytes = u64::try_from(rewritten_wire.len()).unwrap_or(u64::MAX);
        metrics.wire_input_bytes = Some(input_bytes);
        metrics.wire_output_bytes = Some(output_bytes);
        metrics.wire_bytes_delta = signed_delta(input_bytes, output_bytes);
        Self {
            metrics,
            body: Some(rewritten_wire),
        }
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
            evaluated_spans: 0,
            evaluated_input_bytes: 0,
            evaluated_candidate_bytes: 0,
            recovery_verified: false,
            deterministic: false,
            original_fingerprint,
            rewritten_fingerprint: None,
            first_modified_offset: None,
            preserved_prefix_bytes: None,
            wire_input_bytes: None,
            wire_output_bytes: None,
            wire_bytes_delta: None,
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
    let mut evaluated_input_bytes = 0_u64;
    let mut evaluated_candidate_bytes = 0_u64;
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
        evaluated_input_bytes =
            evaluated_input_bytes.saturating_add(u64::try_from(raw.len()).unwrap_or(u64::MAX));
        evaluated_candidate_bytes = evaluated_candidate_bytes
            .saturating_add(u64::try_from(replacement.len()).unwrap_or(u64::MAX));
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
        result.metrics.evaluated_spans = evaluated_spans;
        result.metrics.evaluated_input_bytes = evaluated_input_bytes;
        result.metrics.evaluated_candidate_bytes = evaluated_candidate_bytes;
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
            evaluated_spans,
            evaluated_input_bytes,
            evaluated_candidate_bytes,
            recovery_verified: true,
            deterministic: true,
            original_fingerprint,
            rewritten_fingerprint: Some(digest(&body)),
            first_modified_offset: (prefix < input.len())
                .then_some(u64::try_from(prefix).unwrap_or(u64::MAX)),
            preserved_prefix_bytes: Some(u64::try_from(prefix).unwrap_or(u64::MAX)),
            wire_input_bytes: None,
            wire_output_bytes: None,
            wire_bytes_delta: None,
        },
        body: Some(body.into_boxed_slice()),
    }
}

/// Rewrites selected plain-text `ToolResult` spans with the provider-readable Search projection.
///
/// The span itself remains a normal Responses JSON string. The transformation is semantic-search
/// lossless (all parsed matches remain present), while Tracepress retains an in-memory exact
/// recovery copy for the active safety gate. Any malformed, unsupported, non-deterministic, or
/// non-recoverable span fails open to the original request.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "the active adapter keeps validation, recovery, and fail-open gates in one bounded state machine"
)]
pub fn rewrite_search_projection(
    input: &[u8],
    spans: &[ActiveTextSpan],
    limits: &CompressionLimits,
) -> ActiveRewrite {
    let original_fingerprint = digest(input);
    let base = |status| ActiveRewrite {
        metrics: ActiveRewriteMetrics {
            compressor_id: "search.result_projection".to_owned(),
            compressor_version: 1,
            status,
            input_bytes: u64::try_from(input.len()).unwrap_or(u64::MAX),
            output_bytes: None,
            bytes_delta: None,
            rewrites: 0,
            evaluated_spans: 0,
            evaluated_input_bytes: 0,
            evaluated_candidate_bytes: 0,
            recovery_verified: false,
            deterministic: false,
            original_fingerprint,
            rewritten_fingerprint: None,
            first_modified_offset: None,
            preserved_prefix_bytes: None,
            wire_input_bytes: None,
            wire_output_bytes: None,
            wire_bytes_delta: None,
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
    let mut applicable_spans = 0_u32;
    let mut all_deterministic = true;
    let mut all_recovery_verified = true;
    let mut evaluated_input_bytes = 0_u64;
    let mut evaluated_candidate_bytes = 0_u64;

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
        let content_len = u64::try_from(content.len()).unwrap_or(u64::MAX);
        work = work.saturating_add(content_len.saturating_mul(2));
        if work > limits.max_shadow_work_units || content_len > limits.max_candidate_input_bytes {
            terminal = Some(ActiveRewriteStatus::ResourceLimit);
            continue;
        }
        let first = reduce_search_content(&content, limits);
        let second = reduce_search_content(&content, limits);
        let first_visible = first.visible.clone();
        let second_visible = second.visible.clone();
        let first_recovered = first.recovered.clone();
        let second_recovered = second.recovered.clone();
        if first_visible != second_visible || first_recovered != second_recovered {
            all_deterministic = false;
            terminal = Some(ActiveRewriteStatus::InternalError);
            continue;
        }
        evaluated_spans = evaluated_spans.saturating_add(1);
        evaluated_input_bytes =
            evaluated_input_bytes.saturating_add(u64::try_from(raw.len()).unwrap_or(u64::MAX));
        let status = first.status;
        if !matches!(status, ReductionStatus::Applicable) {
            terminal = Some(match status {
                ReductionStatus::NotApplicable => ActiveRewriteStatus::NotApplicable,
                ReductionStatus::NoImprovement => ActiveRewriteStatus::NoImprovement,
                ReductionStatus::ResourceLimit => ActiveRewriteStatus::ResourceLimit,
                ReductionStatus::InvalidInput => ActiveRewriteStatus::InvalidInput,
                ReductionStatus::InternalError | ReductionStatus::Applicable => {
                    ActiveRewriteStatus::InternalError
                }
            });
            continue;
        }
        applicable_spans = applicable_spans.saturating_add(1);
        let Some(candidate) = first_visible else {
            terminal = Some(ActiveRewriteStatus::InternalError);
            continue;
        };
        let Some(recovered) = first_recovered else {
            all_recovery_verified = false;
            terminal = Some(ActiveRewriteStatus::RecoveryFailed);
            continue;
        };
        if digest(&recovered) != digest(&content) || !first.recovery_verified {
            all_recovery_verified = false;
            terminal = Some(ActiveRewriteStatus::RecoveryFailed);
            continue;
        }
        let replacement = if span.encoded_string {
            let Ok(candidate_text) = std::str::from_utf8(&candidate) else {
                terminal = Some(ActiveRewriteStatus::InvalidInput);
                continue;
            };
            let Ok(encoded) = serde_json::to_vec(candidate_text) else {
                terminal = Some(ActiveRewriteStatus::InternalError);
                continue;
            };
            encoded
        } else {
            candidate
        };
        evaluated_candidate_bytes = evaluated_candidate_bytes
            .saturating_add(u64::try_from(replacement.len()).unwrap_or(u64::MAX));
        if u64::try_from(replacement.len()).unwrap_or(u64::MAX) > limits.max_candidate_output_bytes
        {
            terminal = Some(ActiveRewriteStatus::ResourceLimit);
            continue;
        }
        if replacement.len() < raw.len() {
            replacements.push((start, end, replacement));
        } else {
            terminal = Some(ActiveRewriteStatus::NoImprovement);
        }
    }

    if replacements.is_empty() {
        let status = terminal.unwrap_or(ActiveRewriteStatus::NoImprovement);
        let mut result = base(status);
        result.metrics.deterministic = evaluated_spans > 0 && all_deterministic;
        result.metrics.recovery_verified = applicable_spans > 0 && all_recovery_verified;
        result.metrics.evaluated_spans = evaluated_spans;
        result.metrics.evaluated_input_bytes = evaluated_input_bytes;
        result.metrics.evaluated_candidate_bytes = evaluated_candidate_bytes;
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
            compressor_id: "search.result_projection".to_owned(),
            compressor_version: 1,
            status: ActiveRewriteStatus::Rewritten,
            input_bytes,
            output_bytes: Some(output_bytes),
            bytes_delta: signed_delta(input_bytes, output_bytes),
            rewrites: u32::try_from(replacements.len()).unwrap_or(u32::MAX),
            evaluated_spans,
            evaluated_input_bytes,
            evaluated_candidate_bytes,
            recovery_verified: applicable_spans > 0 && all_recovery_verified,
            deterministic: evaluated_spans > 0 && all_deterministic,
            original_fingerprint,
            rewritten_fingerprint: Some(digest(&body)),
            first_modified_offset: (prefix < input.len())
                .then_some(u64::try_from(prefix).unwrap_or(u64::MAX)),
            preserved_prefix_bytes: Some(u64::try_from(prefix).unwrap_or(u64::MAX)),
            wire_input_bytes: None,
            wire_output_bytes: None,
            wire_bytes_delta: None,
        },
        body: Some(body.into_boxed_slice()),
    }
}

/// Transient result of evaluating either raw Search text or a provider-native JSON envelope.
/// It intentionally has no serialization boundary.
struct SearchContentProjection {
    status: ReductionStatus,
    visible: Option<Vec<u8>>,
    recovered: Option<Box<[u8]>>,
    recovery_verified: bool,
}

fn reduce_search_content(input: &[u8], limits: &CompressionLimits) -> SearchContentProjection {
    let reducer = SearchResultReducer;
    let direct = reducer.reduce(input, limits);
    let direct_result = SearchContentProjection {
        status: direct.metrics().status,
        visible: direct.visible().map(ToOwned::to_owned),
        recovered: direct.recover(),
        recovery_verified: direct.metrics().recovery_verified,
    };
    if !matches!(direct_result.status, ReductionStatus::NotApplicable) {
        return direct_result;
    }
    reduce_search_json_envelope(input, limits).unwrap_or(direct_result)
}

/// Finds one provider-readable Search payload inside a valid JSON envelope and replaces only its
/// JSON string token. The envelope formatting and all non-Search fields remain byte-exact.
fn reduce_search_json_envelope(
    input: &[u8],
    limits: &CompressionLimits,
) -> Option<SearchContentProjection> {
    if u64::try_from(input.len()).ok()? > limits.max_candidate_input_bytes
        || u64::try_from(input.len()).ok()? > limits.max_shadow_memory_bytes
    {
        return Some(SearchContentProjection {
            status: ReductionStatus::ResourceLimit,
            visible: None,
            recovered: None,
            recovery_verified: false,
        });
    }
    let ranges = match json_string_token_ranges(input, limits.max_candidates_per_block) {
        Ok(ranges) => ranges,
        Err(status) => {
            return Some(SearchContentProjection {
                status,
                visible: None,
                recovered: None,
                recovery_verified: false,
            });
        }
    };
    let reducer = SearchResultReducer;
    let mut applicable = None;
    for (start, end) in ranges {
        let raw = input.get(start..end)?;
        let decoded = serde_json::from_slice::<String>(raw).ok()?;
        let candidate = reducer.reduce(decoded.as_bytes(), limits);
        if !matches!(candidate.metrics().status, ReductionStatus::Applicable) {
            continue;
        }
        // More than one Search-like string is ambiguous. Fail closed rather than rewriting an
        // envelope whose semantic ownership we cannot prove from metadata alone.
        if applicable.is_some() {
            return Some(SearchContentProjection {
                status: ReductionStatus::NotApplicable,
                visible: None,
                recovered: None,
                recovery_verified: false,
            });
        }
        let visible = candidate.visible()?.to_vec();
        let recovered = candidate.recover()?;
        if digest(&recovered) != digest(decoded.as_bytes())
            || !candidate.metrics().recovery_verified
        {
            return Some(SearchContentProjection {
                status: ReductionStatus::InternalError,
                visible: None,
                recovered: None,
                recovery_verified: false,
            });
        }
        let rendered = std::str::from_utf8(&visible).ok()?;
        let encoded = serde_json::to_vec(rendered).ok()?;
        applicable = Some((start, end, encoded));
    }
    let Some((start, end, encoded)) = applicable else {
        return Some(SearchContentProjection {
            status: ReductionStatus::NotApplicable,
            visible: None,
            recovered: None,
            recovery_verified: false,
        });
    };
    if encoded.len() >= end.saturating_sub(start) {
        return Some(SearchContentProjection {
            status: ReductionStatus::NoImprovement,
            visible: None,
            recovered: None,
            recovery_verified: false,
        });
    }
    let mut visible = Vec::with_capacity(
        input
            .len()
            .saturating_sub(end.saturating_sub(start))
            .saturating_add(encoded.len()),
    );
    visible.extend_from_slice(input.get(..start)?);
    visible.extend_from_slice(&encoded);
    visible.extend_from_slice(input.get(end..)?);
    let visible_bytes = u64::try_from(visible.len()).ok()?;
    if visible_bytes > limits.max_candidate_output_bytes
        || visible_bytes.saturating_add(u64::try_from(input.len()).ok()?)
            > limits.max_shadow_memory_bytes
    {
        return Some(SearchContentProjection {
            status: ReductionStatus::ResourceLimit,
            visible: None,
            recovered: None,
            recovery_verified: false,
        });
    }
    Some(SearchContentProjection {
        status: ReductionStatus::Applicable,
        visible: Some(visible),
        recovered: Some(input.to_vec().into_boxed_slice()),
        recovery_verified: true,
    })
}

/// Returns byte ranges for every JSON string token after validating the complete document.
/// This is deliberately lexical: the original JSON envelope is retained byte-for-byte except
/// for the one selected value token, so serializing a `Value` can never normalize its shape.
fn json_string_token_ranges(
    input: &[u8],
    maximum: u32,
) -> Result<Vec<(usize, usize)>, ReductionStatus> {
    if serde_json::from_slice::<serde_json::Value>(input).is_err() {
        return Err(ReductionStatus::InvalidInput);
    }
    let mut ranges = Vec::new();
    let mut index = 0_usize;
    while index < input.len() {
        if input.get(index).copied() != Some(b'"') {
            index = index.saturating_add(1);
            continue;
        }
        let start = index;
        index = index.saturating_add(1);
        let mut escaped = false;
        loop {
            let Some(byte) = input.get(index).copied() else {
                return Err(ReductionStatus::InvalidInput);
            };
            index = index.saturating_add(1);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                ranges.push((start, index));
                if u32::try_from(ranges.len()).unwrap_or(u32::MAX) > maximum {
                    return Err(ReductionStatus::ResourceLimit);
                }
                break;
            }
        }
    }
    Ok(ranges)
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
        assert_eq!(result.metrics().evaluated_spans, 1);
        assert!(result.metrics().output_bytes.unwrap_or_default() < request.len() as u64);
        let body = result.body().expect("rewritten body");
        let value: serde_json::Value = serde_json::from_slice(body).expect("rewritten JSON");
        assert_eq!(
            value["input"][0]["output"],
            serde_json::Value::String(r#"{"name":"a","size":10}"#.to_owned())
        );
    }

    #[test]
    fn search_projection_rewrites_grouped_matches_and_recovers_exact_text() {
        let request = br#"{"input":[{"type":"function_call_output","output":"src/a.rs:10:foo\nsrc/a.rs:11:bar\nsrc/a.rs:12:baz\nsrc/b.rs:2:foo\nsrc/b.rs:3:bar\nsrc/b.rs:4:baz\n"}]}"#;
        let span = output_span(request);
        let text_span =
            ActiveTextSpan::new(span.start, span.end, span.encoded_string).expect("text span");
        let result = rewrite_search_projection(request, &[text_span], &limits());
        assert_eq!(result.metrics().status, ActiveRewriteStatus::Rewritten);
        assert!(result.metrics().recovery_verified);
        assert!(result.metrics().deterministic);
        assert!(result.metrics().output_bytes.unwrap_or_default() < request.len() as u64);
        let body = result.body().expect("rewritten body");
        let value: serde_json::Value = serde_json::from_slice(body).expect("rewritten JSON");
        let projected = value["input"][0]["output"]
            .as_str()
            .expect("projected search text");
        assert!(projected.starts_with("[search results]\nsrc/a.rs:\n"));
        assert!(projected.contains("src/b.rs:\n"));
    }

    #[test]
    fn search_projection_rewrites_one_search_string_inside_json_envelope() {
        let request = br#"{"input":[{"type":"function_call_output","output":"{\"kind\":\"shell_result\",\"output\":\"src/a.rs:10:foo\\nsrc/a.rs:11:bar\\nsrc/a.rs:12:baz\\nsrc/a.rs:13:qux\\nsrc/a.rs:14:quux\\nsrc/b.rs:2:foo\\nsrc/b.rs:3:bar\\nsrc/b.rs:4:baz\\nsrc/b.rs:5:qux\\nsrc/b.rs:6:quux\\n\",\"meta\":\"unchanged\"}"}]}"#;
        let span = output_span(request);
        let text_span =
            ActiveTextSpan::new(span.start, span.end, span.encoded_string).expect("text span");
        let result = rewrite_search_projection(request, &[text_span], &limits());
        assert_eq!(result.metrics().status, ActiveRewriteStatus::Rewritten);
        assert!(result.metrics().recovery_verified);
        assert!(result.metrics().deterministic);
        assert_eq!(result.metrics().evaluated_spans, 1);
        let body = result.body().expect("rewritten body");
        let value: serde_json::Value = serde_json::from_slice(body).expect("rewritten request");
        let envelope: serde_json::Value = serde_json::from_str(
            value["input"][0]["output"]
                .as_str()
                .expect("encoded envelope"),
        )
        .expect("rewritten envelope");
        assert_eq!(envelope["kind"], "shell_result");
        assert_eq!(envelope["meta"], "unchanged");
        assert!(
            envelope["output"]
                .as_str()
                .is_some_and(|value| value.starts_with("[search results]\nsrc/a.rs:\n"))
        );
    }

    #[test]
    fn search_projection_fails_open_for_ambiguous_json_envelope() {
        let request = br#"{"input":[{"type":"function_call_output","output":"{\"first\":\"src/a.rs:10:foo\\nsrc/a.rs:11:bar\\n\",\"second\":\"src/b.rs:2:foo\\nsrc/b.rs:3:bar\\n\"}"}]}"#;
        let span = output_span(request);
        let text_span =
            ActiveTextSpan::new(span.start, span.end, span.encoded_string).expect("text span");
        let result = rewrite_search_projection(request, &[text_span], &limits());
        assert_eq!(result.metrics().status, ActiveRewriteStatus::NotApplicable);
        assert!(result.body().is_none());
        assert_eq!(result.metrics().evaluated_spans, 1);
    }

    #[test]
    fn reframes_metrics_and_records_wire_representation() {
        let request = br#"{"output":"{ \"name\": \"a\" }"}"#;
        let span = output_span(request);
        let result = rewrite_json_minify(request, &[span], &limits());
        let reframed = result.reframe(
            b"wire-original-long",
            b"wire-short".to_vec().into_boxed_slice(),
        );
        assert_eq!(reframed.metrics().status, ActiveRewriteStatus::Rewritten);
        assert_eq!(reframed.metrics().input_bytes, request.len() as u64);
        assert!(reframed.metrics().output_bytes.unwrap_or_default() < request.len() as u64);
        assert_eq!(reframed.metrics().wire_input_bytes, Some(18));
        assert_eq!(reframed.metrics().wire_output_bytes, Some(10));
        assert_eq!(reframed.metrics().wire_bytes_delta, Some(8));
        assert!(reframed.body().is_some());

        let expanded_wire = reframed.reframe(b"wire", b"wire-longer".to_vec().into_boxed_slice());
        assert_eq!(
            expanded_wire.metrics().status,
            ActiveRewriteStatus::Rewritten
        );
        assert_eq!(expanded_wire.metrics().wire_input_bytes, Some(4));
        assert_eq!(expanded_wire.metrics().wire_output_bytes, Some(11));
        assert_eq!(expanded_wire.metrics().wire_bytes_delta, Some(-7));
        assert!(expanded_wire.body().is_some());

        let no_input = br#"{"output":"{\"name\":\"a\"}"}"#;
        let no_improvement = rewrite_json_minify(no_input, &[output_span(no_input)], &limits());
        assert_eq!(
            no_improvement.metrics().status,
            ActiveRewriteStatus::NoImprovement
        );
        assert_eq!(no_improvement.metrics().evaluated_spans, 1);
        assert_eq!(
            no_improvement.metrics().evaluated_input_bytes,
            no_improvement.metrics().evaluated_candidate_bytes
        );
        let untouched = no_improvement.reframe(b"wire", b"wire-short".to_vec().into_boxed_slice());
        assert!(untouched.body().is_none());
        assert_eq!(untouched.metrics().wire_input_bytes, None);
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
