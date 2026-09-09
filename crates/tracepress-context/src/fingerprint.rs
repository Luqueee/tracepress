//! Local-only exact and semantic fingerprints for explicit context blocks.

use std::fmt;

use crate::{
    ContextAnalysisLimits, ContextBlockKind, ContextDigest, ContextRole, JsonValueKind, RawSpan,
    SemanticFingerprint, decode_json_string,
};

/// Current ruleset for decoded semantic fingerprints.
pub const SEMANTIC_FINGERPRINT_VERSION: u32 = 1;

/// Everything needed to decide whether a semantic fingerprint is safe to produce.
#[allow(
    clippy::exhaustive_structs,
    reason = "every semantic safety input is explicit at the fingerprint boundary"
)]
#[derive(Clone, Copy)]
pub struct SemanticFingerprintInput<'request> {
    /// Exact observed request bytes containing `value_span`.
    pub request: &'request [u8],
    /// Canonical kind assigned by the context extractor.
    pub block_kind: ContextBlockKind,
    /// Conversational role assigned by the context extractor.
    pub role: ContextRole,
    /// Exact JSON value span carrying the block's text.
    pub value_span: RawSpan,
    /// Structural kind of `value_span` in the raw-span index.
    pub value_kind: JsonValueKind,
    /// Whether the block's indexed subtree contains a duplicate object key.
    pub duplicate_key_in_subtree: bool,
    /// Semantic ruleset to include in the digest.
    pub fingerprint_version: u32,
    /// Inspection bounds governing JSON-string decoding.
    pub limits: ContextAnalysisLimits,
}

impl fmt::Debug for SemanticFingerprintInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticFingerprintInput")
            .field("block_kind", &self.block_kind)
            .field("role", &self.role)
            .field("value_span", &self.value_span)
            .field("value_kind", &self.value_kind)
            .field("duplicate_key_in_subtree", &self.duplicate_key_in_subtree)
            .field("fingerprint_version", &self.fingerprint_version)
            .finish_non_exhaustive()
    }
}

/// Hashes the exact bytes delimited by `span`.
///
/// An invalid or unavailable span produces `None`; it is never replaced by a digest of empty bytes.
#[must_use]
pub fn exact_fingerprint(request: &[u8], span: RawSpan) -> Option<ContextDigest> {
    span.slice(request).map(ContextDigest::from_bytes)
}

/// Hashes decoded text where this version defines semantic equivalence.
///
/// Equivalence is defined only for JSON-string values classified as text, message text, or a tool
/// result. Containers, duplicate-key subtrees, unsupported block kinds, malformed strings, and
/// strings beyond the inspection bound are refused rather than normalized approximately.
#[must_use]
pub fn semantic_fingerprint(input: SemanticFingerprintInput<'_>) -> Option<SemanticFingerprint> {
    if input.duplicate_key_in_subtree
        || input.value_kind != JsonValueKind::String
        || !matches!(
            input.block_kind,
            ContextBlockKind::Text | ContextBlockKind::Message | ContextBlockKind::ToolResult
        )
    {
        return None;
    }

    let decoded = decode_json_string(input.request, input.value_span, input.limits).ok()?;
    let kind = input.block_kind.as_wire_str().as_bytes();
    let role = input.role.as_wire_str().as_bytes();
    let mut material = Vec::with_capacity(
        size_of::<u32>()
            .saturating_add(kind.len())
            .saturating_add(role.len())
            .saturating_add(decoded.len()),
    );
    material.extend_from_slice(&input.fingerprint_version.to_be_bytes());
    material.extend_from_slice(kind);
    material.extend_from_slice(role);
    material.extend_from_slice(decoded.as_bytes());

    Some(SemanticFingerprint {
        fingerprint_version: input.fingerprint_version,
        digest: ContextDigest::from_bytes(&material),
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "invalid fixed test fixtures are fatal test-construction bugs"
)]
mod tests {
    use crate::ContextAnalysisLimitValues;

    use super::*;

    fn limits_with_string_bound(bound: u64) -> ContextAnalysisLimits {
        ContextAnalysisLimits::new(ContextAnalysisLimitValues {
            max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
            max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
            max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
            max_string_bytes_inspected: bound,
            max_analysis_work_units: 2_000,
            max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
            max_batches: ContextAnalysisLimits::MAX_BATCHES,
        })
        .expect("limits")
    }

    fn span(bytes: &[u8]) -> RawSpan {
        RawSpan::new(0, u64::try_from(bytes.len()).expect("length")).expect("span")
    }

    fn input(bytes: &[u8], kind: ContextBlockKind, version: u32) -> SemanticFingerprintInput<'_> {
        SemanticFingerprintInput {
            request: bytes,
            block_kind: kind,
            role: ContextRole::User,
            value_span: span(bytes),
            value_kind: JsonValueKind::String,
            duplicate_key_in_subtree: false,
            fingerprint_version: version,
            limits: limits_with_string_bound(ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED),
        }
    }

    #[test]
    fn exact_fingerprint_uses_only_raw_span_bytes() {
        let request = br#"{"text":"same"}"#;
        let value = RawSpan::new(8, 14).expect("span");
        assert_eq!(
            exact_fingerprint(request, value),
            exact_fingerprint(request, value)
        );
        assert_eq!(
            exact_fingerprint(request, value),
            Some(ContextDigest::from_bytes(br#""same""#))
        );
    }

    #[test]
    fn escaping_changes_exact_but_not_semantic_identity() {
        let plain = br#""a""#;
        let escaped = br#""\u0061""#;
        assert_ne!(
            exact_fingerprint(plain, span(plain)),
            exact_fingerprint(escaped, span(escaped))
        );
        assert_eq!(
            semantic_fingerprint(input(plain, ContextBlockKind::Text, 1)),
            semantic_fingerprint(input(escaped, ContextBlockKind::Text, 1))
        );
    }

    #[test]
    fn version_is_part_of_semantic_identity() {
        let text = br#""same""#;
        assert_ne!(
            semantic_fingerprint(input(text, ContextBlockKind::Text, 1)),
            semantic_fingerprint(input(text, ContextBlockKind::Text, 2))
        );
    }

    #[test]
    fn unsafe_or_undefined_equivalence_is_refused() {
        let text = br#""same""#;
        let mut candidate = input(text, ContextBlockKind::Text, 1);
        candidate.duplicate_key_in_subtree = true;
        assert_eq!(semantic_fingerprint(candidate), None);

        candidate.duplicate_key_in_subtree = false;
        candidate.value_kind = JsonValueKind::Object;
        assert_eq!(semantic_fingerprint(candidate), None);

        candidate.value_kind = JsonValueKind::String;
        candidate.block_kind = ContextBlockKind::ToolCall;
        assert_eq!(semantic_fingerprint(candidate), None);
    }

    #[test]
    fn decode_beyond_inspection_bound_is_refused() {
        let text = br#""long""#;
        let mut candidate = input(text, ContextBlockKind::Text, 1);
        candidate.limits = limits_with_string_bound(3);
        assert_eq!(semantic_fingerprint(candidate), None);
    }

    #[test]
    fn debug_never_emits_content() {
        let candidate = input(br#""private-needle""#, ContextBlockKind::Text, 1);
        let debug = format!("{candidate:?}");
        assert!(!debug.contains("private-needle"));
    }
}
