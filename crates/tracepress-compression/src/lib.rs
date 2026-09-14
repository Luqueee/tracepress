//! Bounded, deterministic shadow-compression evaluation.
//!
//! This crate has no forwarding, credential, database, or filesystem dependencies. Candidate
//! bytes and recovery material exist only for one evaluation and are omitted from [`Debug`].

#![allow(
    missing_docs,
    clippy::arithmetic_side_effects,
    clippy::exhaustive_enums,
    clippy::exhaustive_structs,
    clippy::too_many_arguments,
    reason = "the compressor contract is an explicit measurement DTO and stateless compressors are intentionally constructible unit types"
)]

mod active;
mod json;
mod quality;
mod reduction;
mod text;

use std::{fmt, num::NonZeroU64, time::Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub use active::{
    ActiveJsonSpan, ActiveRewrite, ActiveRewriteMetrics, ActiveRewriteStatus, rewrite_json_minify,
};
pub use json::{
    JsonCompactRecords, JsonKeyElision, JsonMinify, JsonNoop, JsonReadableTable,
    JsonRepeatedSubtree, JsonTabular,
};
pub use quality::{
    ExpectedArtifactKind, QualityAssignment, QualityEvaluatorKind, QualityExperimentArm,
    QualityOutcome, QualityOutcomeStatus, QualitySessionMetrics, QualityTask, ToolFamily,
};
pub use reduction::{
    JsonEmptyNoiseFieldReducer, JsonRepeatedValueReducer, ReductionCandidate, ReductionMetrics,
    ReductionPolicyDecision, ReductionStatus, ShellDiagnosticProjectionReducer,
    TOOL_AWARE_REDUCTION_VERSION, ToolResultReducer,
};
pub use text::{
    TextLogPrefixFold, TextNoop, TextReadableBlockFold, TextReadableLineFold, TextRepeatedLine,
    TextRepeatedRun,
};

/// Phase 4.1 compressor contract version.
pub const SHADOW_COMPRESSION_VERSION: u32 = 1;

/// Explicit policy for structurally unknown content.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum UnknownTransformPolicy {
    /// Unknown content is never transformed.
    Never,
}

/// Provenance required for the narrowly scoped Phase 4.1 targets.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum BlockOrigin {
    /// Tool-produced result data.
    ToolGenerated,
    /// User-authored content.
    HumanAuthored,
    /// Agent-authored content.
    AgentGenerated,
    /// Tool schema content.
    ToolSchema,
    /// Provider-managed content.
    ProviderManaged,
    /// Any other known provenance.
    Other,
}

/// Structural context kind required for candidate applicability.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum BlockKind {
    /// Tool result.
    ToolResult,
    /// Any other block kind.
    Other,
}

/// Locally detected content kind required for candidate applicability.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum DetectedKind {
    /// Valid JSON.
    Json,
    /// Plain text.
    PlainText,
    /// Unknown content, which is never transformed.
    Unknown,
    /// Any other detected kind.
    Other,
}

/// Metadata-only description of one context block.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct BlockMetadata {
    /// Block provenance.
    pub origin: BlockOrigin,
    /// Structural kind.
    pub kind: BlockKind,
    /// Locally detected content kind.
    pub detected_kind: DetectedKind,
    /// Existing local token estimate.
    pub input_estimated_tokens: Option<u64>,
    /// Block start within the analyzed request.
    pub request_offset: u64,
    /// Total analyzed request bytes.
    pub request_analysis_bytes: u64,
    /// Cross-snapshot occurrence count when known.
    pub persistence: Option<u64>,
    /// Whether exact repetition was observed.
    pub exact_repetition: Option<bool>,
}

impl BlockMetadata {
    /// Creates the complete metadata-only applicability input.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the metadata contract keeps orthogonal evidence explicit"
    )]
    pub const fn new(
        origin: BlockOrigin,
        kind: BlockKind,
        detected_kind: DetectedKind,
        input_estimated_tokens: Option<u64>,
        request_offset: u64,
        request_analysis_bytes: u64,
        persistence: Option<u64>,
        exact_repetition: Option<bool>,
    ) -> Self {
        Self {
            origin,
            kind,
            detected_kind,
            input_estimated_tokens,
            request_offset,
            request_analysis_bytes,
            persistence,
            exact_repetition,
        }
    }

    /// Returns whether this is the only JSON target allowed in Phase 4.1.
    #[must_use]
    pub const fn is_tool_result_json(self) -> bool {
        matches!(self.origin, BlockOrigin::ToolGenerated)
            && matches!(self.kind, BlockKind::ToolResult)
            && matches!(self.detected_kind, DetectedKind::Json)
    }

    /// Returns whether this is the only plain-text target allowed in Phase 4.1.
    #[must_use]
    pub const fn is_tool_result_plain_text(self) -> bool {
        matches!(self.origin, BlockOrigin::ToolGenerated)
            && matches!(self.kind, BlockKind::ToolResult)
            && matches!(self.detected_kind, DetectedKind::PlainText)
    }
}

/// Hard bounds for one shadow evaluation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CompressionLimits {
    /// Maximum bytes accepted from one block.
    pub max_candidate_input_bytes: u64,
    /// Maximum candidate bytes produced by one compressor.
    pub max_candidate_output_bytes: u64,
    /// Maximum compressor variants evaluated for one block.
    pub max_candidates_per_block: u32,
    /// Maximum deterministic parser/transform operations.
    pub max_shadow_work_units: u64,
    /// Maximum elapsed wall time for one compressor.
    pub max_shadow_wall_time_ms: NonZeroU64,
    /// Maximum candidate plus recovery working memory.
    pub max_shadow_memory_bytes: u64,
}

impl Default for CompressionLimits {
    fn default() -> Self {
        Self {
            max_candidate_input_bytes: 1_048_576,
            max_candidate_output_bytes: 1_048_576,
            // The Phase 4.4 set contains seven JSON and six plain-text controls/candidates,
            // plus two generic and one family-specific shadow reducer.
            max_candidates_per_block: 16,
            max_shadow_work_units: 2_000_000,
            max_shadow_wall_time_ms: NonZeroU64::new(100).unwrap_or(NonZeroU64::MIN),
            max_shadow_memory_bytes: 4_194_304,
        }
    }
}

/// Stable compressor identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct CompressorId(pub &'static str);

/// Stable compressor implementation version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompressorVersion(pub u32);

/// Explicit terminal candidate state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum CandidateStatus {
    /// A smaller, valid candidate was produced.
    Applicable,
    /// Metadata or shape is outside the compressor target.
    NotApplicable,
    /// The valid candidate failed a never-worse gate.
    NoImprovement,
    /// An explicit work, time, input, output, or memory limit was reached.
    ResourceLimit,
    /// Input was invalid for its declared type.
    InvalidInput,
    /// Exact recovery verification failed.
    RecoveryFailed,
    /// Determinism or another internal invariant failed.
    InternalError,
}

/// Structural cache-risk evidence, never a provider-cache prediction.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum CacheRisk {
    /// Modification occurs late in a persistent block.
    Low,
    /// Modification occurs in the middle of the analyzed prefix.
    Medium,
    /// Modification disrupts an early request prefix.
    High,
    /// Required positional evidence was unavailable.
    Unknown,
}

/// Metadata safe for persistence and Observatory transport.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CandidateMetrics {
    pub compressor_id: String,
    pub compressor_version: u32,
    pub status: CandidateStatus,
    pub input_bytes: u64,
    pub output_bytes: Option<u64>,
    pub bytes_delta: Option<i64>,
    pub input_estimated_tokens: Option<u64>,
    pub output_estimated_tokens: Option<u64>,
    pub estimated_token_delta: Option<i64>,
    pub processing_us: u64,
    pub reversible: bool,
    pub recovery_verified: bool,
    pub deterministic: bool,
    pub original_fingerprint: [u8; 32],
    pub candidate_fingerprint: Option<[u8; 32]>,
    pub first_modified_offset: Option<u64>,
    pub preserved_prefix_bytes: Option<u64>,
    pub preserved_prefix_ratio_basis_points: Option<u16>,
    pub cache_risk: CacheRisk,
}

/// Candidate payload retained only inside the bounded shadow worker.
pub struct CompressionCandidate {
    metrics: CandidateMetrics,
    candidate: Option<Box<[u8]>>,
}

impl fmt::Debug for CompressionCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompressionCandidate")
            .field("metrics", &self.metrics)
            .field(
                "candidate_bytes",
                &self.candidate.as_ref().map(|bytes| bytes.len()),
            )
            .finish()
    }
}

impl CompressionCandidate {
    /// Returns persistence-safe measurements.
    #[must_use]
    pub const fn metrics(&self) -> &CandidateMetrics {
        &self.metrics
    }

    #[cfg(test)]
    fn payload(&self) -> Option<&[u8]> {
        self.candidate.as_deref()
    }
}

/// Internal reversible transformation result.
pub struct Transform {
    candidate: Box<[u8]>,
    recovery: Box<[u8]>,
}

impl Transform {
    pub(crate) fn new(candidate: Vec<u8>, recovery: Vec<u8>) -> Self {
        Self {
            candidate: candidate.into_boxed_slice(),
            recovery: recovery.into_boxed_slice(),
        }
    }

    pub(crate) fn candidate_bytes(&self) -> &[u8] {
        &self.candidate
    }

    pub(crate) fn recovery_bytes(&self) -> &[u8] {
        &self.recovery
    }
}

impl fmt::Debug for Transform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Transform")
            .field("candidate_bytes", &self.candidate.len())
            .field("recovery_bytes", &self.recovery.len())
            .finish()
    }
}

/// Typed internal transformation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformError {
    NotApplicable,
    ResourceLimit,
    InvalidInput,
    Internal,
}

/// Deterministic shadow compressor.
pub trait ShadowCompressor: Send + Sync {
    fn id(&self) -> CompressorId;
    fn version(&self) -> CompressorVersion;
    fn supports(&self, metadata: BlockMetadata) -> bool;
    /// Produces candidate bytes and transient recovery metadata.
    ///
    /// # Errors
    /// Returns a typed applicability, validity, resource, or internal failure.
    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError>;
    /// Recovers the exact original bytes from a candidate and its transient metadata.
    ///
    /// # Errors
    /// Returns a typed failure when the candidate is malformed or exceeds resource bounds.
    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError>;
}

/// Applies common bounds, determinism, recovery, and never-worse gates.
#[must_use]
pub fn evaluate(
    compressor: &dyn ShadowCompressor,
    metadata: BlockMetadata,
    input: &[u8],
    limits: &CompressionLimits,
) -> CompressionCandidate {
    evaluate_with_estimator(compressor, metadata, input, limits, |_| None)
}

/// Evaluates a compressor and applies the caller's existing estimator to candidate bytes.
#[must_use]
pub fn evaluate_with_estimator(
    compressor: &dyn ShadowCompressor,
    metadata: BlockMetadata,
    input: &[u8],
    limits: &CompressionLimits,
    estimate: impl Fn(&[u8]) -> Option<u64>,
) -> CompressionCandidate {
    let started = Instant::now();
    let original_fingerprint = digest(input);
    let base = |status| CandidateMetrics {
        compressor_id: compressor.id().0.to_owned(),
        compressor_version: compressor.version().0,
        status,
        input_bytes: u64::try_from(input.len()).unwrap_or(u64::MAX),
        output_bytes: None,
        bytes_delta: None,
        input_estimated_tokens: metadata.input_estimated_tokens,
        output_estimated_tokens: None,
        estimated_token_delta: None,
        processing_us: elapsed_us(started),
        reversible: true,
        recovery_verified: false,
        deterministic: false,
        original_fingerprint,
        candidate_fingerprint: None,
        first_modified_offset: None,
        preserved_prefix_bytes: None,
        preserved_prefix_ratio_basis_points: None,
        cache_risk: CacheRisk::Unknown,
    };
    if !compressor.supports(metadata) {
        return candidate(base(CandidateStatus::NotApplicable), None);
    }
    let input_bytes = u64::try_from(input.len()).unwrap_or(u64::MAX);
    if input_bytes > limits.max_candidate_input_bytes
        || input_bytes > limits.max_shadow_memory_bytes
    {
        return candidate(base(CandidateStatus::ResourceLimit), None);
    }
    let (first, second) = match (
        compressor.transform(input, limits),
        compressor.transform(input, limits),
    ) {
        (Ok(first), Ok(second)) => (first, second),
        (Err(error), _) | (_, Err(error)) => return candidate(base(status_for(error)), None),
    };
    let deterministic = first.candidate == second.candidate && first.recovery == second.recovery;
    let output_bytes = u64::try_from(first.candidate.len()).unwrap_or(u64::MAX);
    let recovery_bytes = u64::try_from(first.recovery.len()).unwrap_or(u64::MAX);
    if output_bytes > limits.max_candidate_output_bytes
        || output_bytes.saturating_add(recovery_bytes) > limits.max_shadow_memory_bytes
        || elapsed_ms(started) > limits.max_shadow_wall_time_ms.get()
    {
        return candidate(base(CandidateStatus::ResourceLimit), None);
    }
    let recovery_verified = compressor
        .recover(&first.candidate, &first.recovery, limits)
        .as_deref()
        .is_ok_and(|recovered| digest(recovered) == original_fingerprint);
    let candidate_fingerprint = digest(&first.candidate);
    let prefix = common_prefix(input, &first.candidate);
    let prefix_u64 = u64::try_from(prefix).unwrap_or(u64::MAX);
    let prefix_ratio = prefix_ratio(metadata, prefix_u64);
    let output_estimated_tokens = metadata
        .input_estimated_tokens
        .and_then(|_| estimate(&first.candidate));
    let token_delta = metadata
        .input_estimated_tokens
        .zip(output_estimated_tokens)
        .and_then(|(input_tokens, output_tokens)| signed_delta(input_tokens, output_tokens));
    let no_token_improvement = metadata
        .input_estimated_tokens
        .zip(output_estimated_tokens)
        .is_some_and(|(input_tokens, output_tokens)| output_tokens >= input_tokens);
    let status = if !deterministic {
        CandidateStatus::InternalError
    } else if !recovery_verified {
        CandidateStatus::RecoveryFailed
    } else if output_bytes >= input_bytes || no_token_improvement {
        CandidateStatus::NoImprovement
    } else {
        CandidateStatus::Applicable
    };
    let metrics = CandidateMetrics {
        compressor_id: compressor.id().0.to_owned(),
        compressor_version: compressor.version().0,
        status,
        input_bytes,
        output_bytes: Some(output_bytes),
        bytes_delta: signed_delta(input_bytes, output_bytes),
        input_estimated_tokens: metadata.input_estimated_tokens,
        output_estimated_tokens,
        estimated_token_delta: token_delta,
        processing_us: elapsed_us(started),
        reversible: true,
        recovery_verified,
        deterministic,
        original_fingerprint,
        candidate_fingerprint: Some(candidate_fingerprint),
        first_modified_offset: (prefix < input.len() || prefix < first.candidate.len())
            .then_some(prefix_u64),
        preserved_prefix_bytes: Some(prefix_u64),
        preserved_prefix_ratio_basis_points: prefix_ratio,
        cache_risk: cache_risk(metadata, prefix_ratio),
    };
    candidate(metrics, Some(first.candidate))
}

const fn candidate(
    metrics: CandidateMetrics,
    candidate: Option<Box<[u8]>>,
) -> CompressionCandidate {
    CompressionCandidate { metrics, candidate }
}

const fn status_for(error: TransformError) -> CandidateStatus {
    match error {
        TransformError::NotApplicable => CandidateStatus::NotApplicable,
        TransformError::ResourceLimit => CandidateStatus::ResourceLimit,
        TransformError::InvalidInput => CandidateStatus::InvalidInput,
        TransformError::Internal => CandidateStatus::InternalError,
    }
}

fn prefix_ratio(metadata: BlockMetadata, prefix: u64) -> Option<u16> {
    if metadata.request_analysis_bytes == 0 {
        return None;
    }
    let basis = metadata
        .request_offset
        .saturating_add(prefix)
        .saturating_mul(10_000)
        .checked_div(metadata.request_analysis_bytes)?
        .min(10_000);
    u16::try_from(basis).ok()
}

fn cache_risk(metadata: BlockMetadata, prefix_basis_points: Option<u16>) -> CacheRisk {
    match prefix_basis_points {
        None => CacheRisk::Unknown,
        Some(value) if value >= 9_000 && metadata.persistence.unwrap_or(0) > 1 => CacheRisk::Low,
        Some(value) if value >= 5_000 => CacheRisk::Medium,
        Some(_) => CacheRisk::High,
    }
}

fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn signed_delta(input: u64, output: u64) -> Option<i64> {
    i64::try_from(i128::from(input) - i128::from(output)).ok()
}

fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

pub(crate) fn digest(input: &[u8]) -> [u8; 32] {
    Sha256::digest(input).into()
}

pub(crate) fn bounded_vec(
    bytes: &[u8],
    limits: &CompressionLimits,
) -> Result<Vec<u8>, TransformError> {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if length > limits.max_candidate_output_bytes || length > limits.max_shadow_memory_bytes {
        return Err(TransformError::ResourceLimit);
    }
    Ok(bytes.to_vec())
}

pub(crate) fn validate_recovery_inputs(
    candidate: &[u8],
    recovery: &[u8],
    limits: &CompressionLimits,
) -> Result<(), TransformError> {
    let candidate_bytes = u64::try_from(candidate.len()).unwrap_or(u64::MAX);
    let recovery_bytes = u64::try_from(recovery.len()).unwrap_or(u64::MAX);
    if candidate_bytes > limits.max_candidate_output_bytes
        || candidate_bytes.saturating_add(recovery_bytes) > limits.max_shadow_memory_bytes
    {
        Err(TransformError::ResourceLimit)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_metadata() -> BlockMetadata {
        BlockMetadata {
            origin: BlockOrigin::ToolGenerated,
            kind: BlockKind::ToolResult,
            detected_kind: DetectedKind::Json,
            input_estimated_tokens: Some(1_000),
            request_offset: 200,
            request_analysis_bytes: 2_000,
            persistence: Some(3),
            exact_repetition: Some(true),
        }
    }

    #[test]
    fn unknown_and_non_target_origins_are_never_transformed() {
        let limits = CompressionLimits::default();
        for detected_kind in [DetectedKind::Unknown, DetectedKind::Json] {
            let mut metadata = json_metadata();
            metadata.detected_kind = detected_kind;
            if detected_kind == DetectedKind::Json {
                metadata.origin = BlockOrigin::HumanAuthored;
            }
            let result = evaluate(&JsonMinify, metadata, br#"{ "ok": true }"#, &limits);
            assert_eq!(result.metrics().status, CandidateStatus::NotApplicable);
            assert!(result.payload().is_none());
        }
    }

    #[test]
    fn noop_control_is_deterministic_and_never_worse() {
        let limits = CompressionLimits::default();
        let first = evaluate(&JsonNoop, json_metadata(), br#"{"ok":true}"#, &limits);
        let second = evaluate(&JsonNoop, json_metadata(), br#"{"ok":true}"#, &limits);
        assert_eq!(first.metrics().status, CandidateStatus::NoImprovement);
        assert_eq!(
            Some(first.metrics().original_fingerprint),
            first.metrics().candidate_fingerprint
        );
        assert_eq!(
            first.metrics().candidate_fingerprint,
            second.metrics().candidate_fingerprint
        );
        assert!(first.metrics().recovery_verified);
        assert!(first.metrics().deterministic);
    }

    #[test]
    fn input_exceeding_memory_budget_is_rejected_before_transform() {
        let limits = CompressionLimits {
            max_shadow_memory_bytes: 2,
            ..CompressionLimits::default()
        };
        let result = evaluate(&JsonNoop, json_metadata(), b"[1]", &limits);
        assert_eq!(result.metrics().status, CandidateStatus::ResourceLimit);
        assert!(result.payload().is_none());
    }

    #[test]
    fn candidate_debug_never_exposes_ephemeral_content() {
        let canary = "tracepress-private-candidate-canary";
        let input = format!(r#"{{ "secret": "{canary}" }}"#);
        let result = evaluate(
            &JsonMinify,
            json_metadata(),
            input.as_bytes(),
            &CompressionLimits::default(),
        );
        let debug = format!("{result:?}");
        assert!(!debug.contains(canary));
        assert!(!debug.contains("secret"));
        assert!(debug.contains("candidate_bytes"));
    }
}
