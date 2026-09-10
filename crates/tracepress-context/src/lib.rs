//! Shadow context analysis domain for Tracepress.
//!
//! This crate answers "what did we send" for one observed provider request: it decomposes the
//! explicit bytes into typed, positioned, measured blocks and records how much of the request is
//! invisible to Tracepress. It observes, classifies, and measures. It never transforms,
//! compresses, rewrites, fetches, or mutates a request, and nothing here participates in
//! forwarding.
//!
//! Provider parsing stays in `tracepress-provider`, which answers the different question of what
//! the provider reported. Keeping the two apart is what lets a request payload be rewritten later
//! without touching usage and lifecycle parsing.
//!
//! The durable and IPC allowlist is structural and statistical: identifiers, kinds, roles,
//! origins, spans, byte counts, fingerprints, estimates, detector results, and bounded metadata.
//! Prompt text, instructions, tool arguments, tool results, tool schema contents, URLs, and file
//! contents are excluded by construction rather than scrubbed, and `Debug` implementations expose
//! kinds, counts, spans, and statuses only.

mod delta;
mod detector;
mod digest;
mod domain;
mod estimator;
mod extractor;
mod features;
mod fingerprint;
mod limits;
mod metadata;
mod reconciliation;
mod span;
mod visibility;
mod visibility_analysis;

pub use extractor::{
    ContextAnalysis, ContextAnalysisLimitReason, ContextAnalysisReason, ContextAnalysisResult,
    ContextBlockDraft, analyze, analyze_responses, analyze_responses_with_lookup,
    analyze_with_lookup,
};
pub use visibility_analysis::{
    ContextVisibilityFacts, NoObservedResponseIds, ObservedResponseIdLookup,
    ProviderStateReference, VisibilityAnalysis, VisibilityObservation, derive_visibility,
    derive_visibility_without_lookup,
};

pub use delta::{
    ContextBlockSummary, ContextDelta, ContextDeltaRequest, ContextDeltaStatus,
    compute_context_delta,
};
pub use detector::{
    BlockContentMetadata, STRUCTURAL_DETECTOR_VERSION, ShadowContentDetector,
    StructuralContentDetector,
};
pub use digest::{ContextDigest, ContextDigestParseError};
pub use domain::{
    ANALYZED_PROVIDER_KIND, ANALYZED_PROVIDER_PROTOCOL, BlockLocator, CONTEXT_ANALYSIS_VERSION,
    ContextAnalysisDropReason, ContextAnalysisStatus, ContextBlockKind, ContextBlockOccurrence,
    ContextOrigin, ContextRole, ContextSnapshot, DetectedContentKind, DetectionConfidence,
    DetectionResult, EstimateConfidence, SemanticFingerprint, TokenEstimate,
};
pub use estimator::{
    EstimateUnavailable, EstimationRequest, StructuralHeuristicEstimator, TokenEstimateAggregate,
    TokenEstimation, TokenEstimator,
};
pub use features::{
    BlockFeatureInput, BlockFeatures, FeatureCoverage, FeatureRatio, OpportunitySignal,
    OpportunitySignalInput, OpportunitySignalSet, derive_opportunity_signals,
    extract_block_features,
};
pub use fingerprint::{
    SEMANTIC_FINGERPRINT_VERSION, SemanticFingerprintInput, exact_fingerprint, semantic_fingerprint,
};
pub use limits::{
    ContextAnalysisLimitField, ContextAnalysisLimitValues, ContextAnalysisLimits,
    ContextAnalysisLimitsError,
};
pub use metadata::{BoundedMetadataText, MetadataTextError};
pub use reconciliation::{ReconciliationStatus, TokenReconciliation};
pub use span::{
    AnalysisClock, JsonStringDecodeError, JsonValueKind, MonotonicClock, RawSpan, RawSpanIndex,
    SpanNode, SpanNodeChildren, SpanNodeId, decode_json_string,
};
pub use visibility::{ContextVisibility, LogicalContextStatus};
