//! Stable, platform-neutral contracts shared by the Observatory API and WASM UI.
#![allow(
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::exhaustive_enums,
    clippy::exhaustive_structs,
    clippy::expect_used,
    clippy::struct_excessive_bools,
    reason = "wire DTOs are exhaustive by contract, include f64 metrics, and tests use explicit fixture expectations"
)]

use serde::{Deserialize, Serialize};

/// Whether a metric comes from the provider, a local estimator, or is absent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricSource {
    /// Reported by the upstream provider.
    ProviderReported,
    /// Estimated locally by Tracepress.
    LocallyEstimated,
    /// The underlying observation is unavailable.
    Unavailable,
}

/// A nullable numeric measurement with explicit provenance.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Metric<T> {
    /// The measurement. `None` must never be rendered as zero.
    pub value: Option<T>,
    /// Measurement provenance.
    pub source: MetricSource,
}

impl<T> Metric<T> {
    /// Creates a metric with explicit provenance.
    #[must_use]
    pub const fn new(value: Option<T>, source: MetricSource) -> Self {
        Self { value, source }
    }
}

/// Standard typed API error payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApiErrorResponse {
    /// Error details safe to expose to the browser.
    pub error: ApiError,
}

/// Browser-safe API error details.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApiError {
    /// Stable machine-readable identifier.
    pub code: String,
    /// Human-readable message without SQL or filesystem details.
    pub message: String,
}

/// Cursor-paginated response.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Page<T> {
    /// Bounded result items.
    pub items: Vec<T>,
    /// Opaque cursor for the next page.
    pub next_cursor: Option<String>,
}

/// Measurement integrity summary shown prominently throughout the UI.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MeasurementQuality {
    /// Overall state such as `healthy` or `degraded`.
    pub status: String,
    /// Context-analysis coverage ratio.
    pub analysis_coverage: Option<f64>,
    /// Provider/context correlation coverage ratio.
    pub correlation_coverage: Option<f64>,
    /// Semantic-estimation coverage ratio.
    pub semantic_coverage: Option<f64>,
    /// Requests that could not be analyzed.
    pub dropped_requests: u64,
    /// Malformed context observations.
    pub malformed_requests: u64,
    /// Complete explanatory messages for degraded states.
    pub reasons: Vec<String>,
}

/// Provider usage totals.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UsageSummary {
    /// Input tokens.
    pub input_tokens: Metric<u64>,
    /// Cached input tokens.
    pub cached_input_tokens: Metric<u64>,
    /// Uncached input tokens.
    pub uncached_input_tokens: Metric<u64>,
    /// Cache ratio in `[0, 1]`.
    pub cache_ratio: Metric<f64>,
    /// Output tokens.
    pub output_tokens: Metric<u64>,
    /// Reasoning output tokens.
    pub reasoning_tokens: Metric<u64>,
}

/// Repetition metrics which must not be described as savings.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepetitionSummary {
    /// Exact repeated locally-estimated token share.
    pub exact_token_share: Metric<f64>,
    /// Semantic repeated locally-estimated token share.
    pub semantic_token_share: Metric<f64>,
    /// Provider-reported cache ratio shown for correlation only.
    pub provider_cache_ratio: Metric<f64>,
}

/// One category in context composition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextCategoryStats {
    /// Stable category key.
    pub name: String,
    /// Number of observed blocks.
    pub block_count: u64,
    /// Raw bytes represented by those blocks.
    pub raw_bytes: u64,
    /// Nullable estimated tokens.
    pub estimated_tokens: Option<u64>,
    /// Share of the estimated subset, not necessarily full context.
    pub estimated_token_share: Option<f64>,
    /// Exact repeated estimated tokens.
    pub exact_repeated_tokens: Option<u64>,
    /// Semantic repeated estimated tokens.
    pub semantic_repeated_tokens: Option<u64>,
    /// Average cross-request persistence.
    pub persistence: Option<f64>,
}

/// Context composition grouped by the selected dimension.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextComposition {
    /// Grouping dimension.
    pub group_by: String,
    /// Category rows.
    pub categories: Vec<ContextCategoryStats>,
    /// Fraction of blocks with token estimates.
    pub estimator_coverage: Option<f64>,
}

/// Observatory overview response.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Overview {
    /// Number of sessions.
    pub sessions: u64,
    /// Number of provider requests.
    pub provider_requests: u64,
    /// Provider usage totals.
    pub usage: UsageSummary,
    /// Context composition by detected kind.
    pub composition: ContextComposition,
    /// Repetition and provider caching comparison.
    pub repetition: RepetitionSummary,
    /// Measurement quality which the UI must not hide.
    pub quality: MeasurementQuality,
}

/// Compact session row.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionSummary {
    /// Stable session UUID.
    pub id: String,
    /// Optional externally certified workload label.
    pub workload: Option<String>,
    /// Provider model observed for the session.
    pub model: Option<String>,
    /// Provider transport.
    pub transport: Option<String>,
    /// Session start time as persisted.
    pub started_at: String,
    /// Session end time; absent while running.
    pub ended_at: Option<String>,
    /// Durable session state.
    pub status: String,
    /// Number of provider requests.
    pub request_count: u64,
    /// Whether any compaction request occurred.
    pub has_compaction: bool,
    /// Provider usage.
    pub usage: UsageSummary,
    /// Estimated visible context on the latest comparable snapshot.
    pub estimated_context_tokens: Metric<u64>,
    /// Exact repetition ratio.
    pub repetition: Metric<f64>,
    /// Semantic estimation coverage.
    pub semantic_coverage: Metric<f64>,
    /// Duration in microseconds.
    pub duration_us: Option<u64>,
    /// Optional measurement identifier when a certified mapping is available.
    pub measurement_id: Option<String>,
}

/// Request row in a session timeline.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProviderRequestSummary {
    /// Provider request UUID.
    pub id: String,
    /// One-based request order within the session.
    pub ordinal: u64,
    /// `turn`, `compaction_v2`, or another persisted kind.
    pub kind: String,
    /// Provider model.
    pub model: Option<String>,
    /// Request/attempt status.
    pub status: String,
    /// Provider usage.
    pub usage: UsageSummary,
    /// Locally estimated explicit context.
    pub estimated_context_tokens: Metric<u64>,
    /// Semantic estimation coverage.
    pub semantic_coverage: Metric<f64>,
    /// Attempt duration.
    pub duration_us: Option<u64>,
}

/// Complete metadata-only session view.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionDetail {
    /// Session summary.
    pub session: SessionSummary,
    /// Most recently observed reasoning effort.
    pub reasoning_effort: Option<String>,
    /// Ordered provider request timeline.
    pub requests: Vec<ProviderRequestSummary>,
    /// Growth samples for charting.
    pub context_growth: Vec<ContextGrowthPoint>,
}

/// One request-order sample for context growth.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextGrowthPoint {
    /// One-based provider request order.
    pub ordinal: u64,
    /// Provider input tokens.
    pub provider_input_tokens: Option<u64>,
    /// Provider cached input tokens.
    pub cached_input_tokens: Option<u64>,
    /// Provider uncached input tokens.
    pub uncached_input_tokens: Option<u64>,
    /// Locally estimated visible context.
    pub estimated_context_tokens: Option<u64>,
}

/// Metadata-only context block.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextBlockSummary {
    /// Stable occurrence UUID.
    pub id: String,
    /// Position within the snapshot.
    pub ordinal: u64,
    /// Context block kind.
    pub kind: String,
    /// Logical role.
    pub role: String,
    /// Origin classification.
    pub origin: String,
    /// Detected content kind.
    pub detected_kind: Option<String>,
    /// Raw byte count only; content is never returned.
    pub raw_bytes: u64,
    /// Locally estimated tokens.
    pub estimated_tokens: Option<u64>,
    /// Whether this exact fingerprint occurred more than once.
    pub exact_repeated: Option<bool>,
    /// Whether this semantic fingerprint occurred more than once.
    pub semantic_repeated: Option<bool>,
    /// Cross-request fingerprint occurrence count.
    pub persistence: Option<u64>,
    /// Truncated hexadecimal fingerprint.
    pub fingerprint_short: Option<String>,
}

/// Context endpoint for one session.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionContext {
    /// Session UUID.
    pub session_id: String,
    /// Latest snapshot UUID.
    pub snapshot_id: Option<String>,
    /// Visibility status.
    pub visibility: Option<String>,
    /// Context composition.
    pub composition: ContextComposition,
    /// Bounded block page.
    pub blocks: Page<ContextBlockSummary>,
}

/// One cell in the Origin x DetectedContentKind matrix.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextMatrixCell {
    /// Context origin.
    pub origin: String,
    /// Detected content kind.
    pub detected_kind: String,
    /// Block count.
    pub block_count: u64,
    /// Raw bytes.
    pub raw_bytes: u64,
    /// Estimated token total.
    pub estimated_tokens: Option<u64>,
    /// Share of all estimated tokens.
    pub estimated_token_share: Option<f64>,
}

/// Global context explorer response.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextExplorer {
    /// Selected grouping.
    pub composition: ContextComposition,
    /// Origin by detected-kind cells.
    pub matrix: Vec<ContextMatrixCell>,
}

/// Unknown-content investigation summary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UnknownSummary {
    /// Unknown blocks.
    pub blocks: u64,
    /// Estimated tokens among estimable unknown blocks.
    pub estimated_tokens: Option<u64>,
    /// Estimator coverage among unknown blocks.
    pub estimator_coverage: Option<f64>,
    /// Raw bytes.
    pub raw_bytes: u64,
    /// Unique exact fingerprints.
    pub unique_exact_fingerprints: u64,
    /// Unique semantic fingerprints.
    pub unique_semantic_fingerprints: u64,
    /// Size percentiles in raw bytes.
    pub size_percentiles: Percentiles,
    /// Average persistence.
    pub persistence: Option<f64>,
}

/// Nullable distribution percentiles.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Percentiles {
    /// Median.
    pub p50: Option<u64>,
    /// 90th percentile.
    pub p90: Option<u64>,
    /// 99th percentile.
    pub p99: Option<u64>,
}

/// Aggregate metrics for one certified workload.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkloadSummary {
    /// Workload label.
    pub workload: String,
    /// Session count.
    pub sessions: u64,
    /// Provider request count.
    pub requests: u64,
    /// Share of provider input tokens.
    pub token_share: Option<f64>,
    /// Estimated detected JSON share.
    pub json_share: Option<f64>,
    /// Estimated detected plain-text share.
    pub plain_share: Option<f64>,
    /// Estimated detected unknown share.
    pub unknown_share: Option<f64>,
    /// Provider cache ratio.
    pub cache_ratio: Option<f64>,
    /// Exact repeated estimated token share.
    pub exact_repetition: Option<f64>,
    /// Semantic repeated estimated token share.
    pub semantic_repetition: Option<f64>,
}

/// Baseline list row.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BaselineSummary {
    /// Baseline identifier.
    pub id: String,
    /// `exploratory`, `legacy_uncertified`, `valid`, or `converged`.
    pub status: String,
    /// Number of sessions in the latest cohort.
    pub sessions: Option<u64>,
    /// Number of provider requests.
    pub requests: Option<u64>,
    /// Latest cohort label.
    pub cohort: Option<String>,
    /// Measurement integrity status.
    pub measurement_integrity: Option<String>,
}

/// One convergence series point from official baseline tooling.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConvergencePoint {
    /// Cohort label such as `N10`.
    pub cohort: String,
    /// JSON estimated share.
    pub json_share: Option<f64>,
    /// Plain-text estimated share.
    pub plain_share: Option<f64>,
    /// Unknown estimated share.
    pub unknown_share: Option<f64>,
    /// Provider cache ratio.
    pub cache_ratio: Option<f64>,
    /// Exact repetition share.
    pub repetition: Option<f64>,
    /// Official tooling stability result.
    pub stable: Option<bool>,
}

/// Baseline detail derived from reproducible report artifacts.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BaselineDetail {
    /// List summary.
    pub summary: BaselineSummary,
    /// Manifest metadata safe for display.
    pub manifest: BaselineManifest,
    /// Provider usage.
    pub usage: Option<UsageSummary>,
    /// Measurement quality.
    pub quality: Option<MeasurementQuality>,
    /// Context composition.
    pub composition: Option<ContextComposition>,
    /// Repetition.
    pub repetition: Option<RepetitionSummary>,
    /// Workload breakdown.
    pub workloads: Vec<WorkloadSummary>,
    /// Official N10/N20/N30/N40 values.
    pub convergence: Vec<ConvergencePoint>,
    /// Official recommendation text when present.
    pub recommendation: Option<String>,
}

/// Safe baseline manifest projection.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BaselineManifest {
    /// Instrumentation commit SHA.
    pub instrument_sha: Option<String>,
    /// Tracepress/runtime commit SHA.
    pub runtime_sha: Option<String>,
    /// Codex version.
    pub codex_version: Option<String>,
    /// Provider model.
    pub model: Option<String>,
    /// Reasoning effort.
    pub reasoning_effort: Option<String>,
    /// Transport.
    pub transport: Option<String>,
}

/// Evidence-only optimization candidate; never a savings claim.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OpportunitySummary {
    /// Content category.
    pub category: String,
    /// Effective presence in estimated tokens.
    pub effective_presence: Option<u64>,
    /// Repeated estimated tokens.
    pub repeated_tokens: Option<u64>,
    /// Average persistence.
    pub persistence: Option<f64>,
    /// Repetition/redundancy factor.
    pub redundancy: Option<f64>,
    /// Confidence in the underlying measurement.
    pub measurement_confidence: Option<f64>,
    /// Existing tooling candidate-priority score.
    pub candidate_priority: Option<f64>,
}

/// Metadata-only summary of one shadow experiment.
#[allow(
    missing_docs,
    reason = "field names are the versioned dashboard wire contract"
)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionExperimentSummary {
    pub id: String,
    pub status: String,
    pub candidate_count: u64,
    pub session_count: u64,
    pub block_count: u64,
    pub runtime_sha: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub quality: CompressionQuality,
}

/// Shadow-only integrity counters. Forwarding mutations must remain zero.
#[allow(
    missing_docs,
    reason = "field names are the versioned dashboard wire contract"
)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionQuality {
    pub forwarding_mutations: u64,
    pub shadow_drops: u64,
    pub shadow_queue_full_drops: u64,
    pub shadow_byte_budget_drops: u64,
    pub shadow_work_budget_drops: u64,
    pub shadow_worker_closed_drops: u64,
    pub shadow_persistence_drops: u64,
    pub recovery_failures: u64,
    pub determinism_failures: u64,
}

/// Aggregated evidence for one compressor implementation.
#[allow(
    missing_docs,
    reason = "field names are the versioned dashboard wire contract"
)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressorSummary {
    pub compressor: String,
    pub version: String,
    pub eligible_blocks: u64,
    pub applicable_blocks: u64,
    pub applicability_basis_points: Option<u16>,
    /// Estimated-token exposure covered by applicable occurrences.
    pub addressable_token_share_basis_points: Option<u16>,
    /// Whether the candidate is readable by a normal provider/model contract.
    pub provider_readability: String,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub byte_reduction: Option<u64>,
    pub candidate_effective_byte_reduction: Option<u64>,
    pub unique_candidate_byte_reduction: Option<u64>,
    pub byte_reduction_basis_points: Option<u16>,
    pub estimated_input_tokens: Option<u64>,
    pub estimated_output_tokens: Option<u64>,
    pub estimated_reduction: Option<u64>,
    pub candidate_effective_estimated_token_reduction: Option<u64>,
    pub unique_candidate_estimated_token_reduction: Option<u64>,
    pub estimated_reduction_basis_points: Option<u16>,
    pub recovery_basis_points: Option<u16>,
    pub deterministic_basis_points: Option<u16>,
    pub processing_p50_us: Option<u64>,
    pub processing_p90_us: Option<u64>,
    pub processing_p95_us: Option<u64>,
    pub processing_p99_us: Option<u64>,
    pub cache_risk_low: u64,
    pub cache_risk_medium: u64,
    pub cache_risk_high: u64,
    pub cache_risk_unknown: u64,
}

/// Complete metadata-only view of one shadow experiment.
#[allow(
    missing_docs,
    reason = "field names are the versioned dashboard wire contract"
)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionExperimentDetail {
    pub summary: CompressionExperimentSummary,
    pub compressor_set_json: String,
    pub limits_json: String,
    pub compressors: Vec<CompressorSummary>,
    pub reduction_histogram: Vec<CompressionHistogramBucket>,
    pub latency_histogram: Vec<CompressionHistogramBucket>,
    pub workload_distribution_available: bool,
}

/// Bounded distribution bucket with integer boundaries.
#[allow(
    missing_docs,
    reason = "field names are the versioned dashboard wire contract"
)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionHistogramBucket {
    pub label: String,
    pub lower_inclusive: u64,
    pub upper_exclusive: Option<u64>,
    pub count: u64,
}

/// Metadata-only candidate row; original and transformed content are intentionally absent.
#[allow(
    missing_docs,
    reason = "field names are the versioned dashboard wire contract"
)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionCandidateSummary {
    pub id: String,
    pub experiment_id: String,
    pub snapshot_id: String,
    pub block_ordinal: u64,
    pub block_kind: String,
    pub origin: String,
    pub detected_kind: Option<String>,
    pub compressor: String,
    pub version: String,
    pub status: String,
    pub input_bytes: u64,
    pub output_bytes: Option<u64>,
    pub byte_reduction: Option<u64>,
    pub input_estimated_tokens: Option<u64>,
    pub output_estimated_tokens: Option<u64>,
    pub estimated_reduction: Option<u64>,
    pub recovery_verified: bool,
    pub deterministic: bool,
    pub latency_us: Option<u64>,
    pub preserved_prefix_bytes: Option<u64>,
    pub preserved_prefix_ratio_basis_points: Option<u16>,
    pub cache_risk: String,
    pub exact_repetition: Option<bool>,
    pub persistence: Option<u64>,
    pub provider_readability: String,
    pub json_root_kind: Option<String>,
    pub json_array_length_bucket: Option<String>,
    pub json_object_key_count_bucket: Option<String>,
    pub json_homogeneity_basis_points: Option<u16>,
    pub json_primitive_cell_ratio_basis_points: Option<u16>,
    pub json_nested_cell_ratio_basis_points: Option<u16>,
    pub text_shape: Option<String>,
}

/// Scheduler accounting summary when certified data exists.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SchedulerSummary {
    /// Whether certified scheduler data is available.
    pub available: bool,
    /// Eligible provider requests.
    pub eligible_requests: Option<u64>,
    /// Admitted analyses.
    pub admitted: Option<u64>,
    /// Dropped analyses.
    pub dropped: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{Metric, MetricSource, SessionSummary, UsageSummary};

    #[test]
    fn dto_roundtrip_preserves_null_instead_of_zero() {
        let value = SessionSummary {
            id: "session-1".to_owned(),
            workload: None,
            model: Some("gpt-test".to_owned()),
            transport: None,
            started_at: "2026-01-01T00:00:00Z".to_owned(),
            ended_at: None,
            status: "running".to_owned(),
            request_count: 0,
            has_compaction: false,
            usage: UsageSummary {
                input_tokens: Metric::new(None, MetricSource::Unavailable),
                cached_input_tokens: Metric::new(None, MetricSource::Unavailable),
                uncached_input_tokens: Metric::new(None, MetricSource::Unavailable),
                cache_ratio: Metric::new(None, MetricSource::Unavailable),
                output_tokens: Metric::new(None, MetricSource::Unavailable),
                reasoning_tokens: Metric::new(None, MetricSource::Unavailable),
            },
            estimated_context_tokens: Metric::new(None, MetricSource::Unavailable),
            repetition: Metric::new(None, MetricSource::Unavailable),
            semantic_coverage: Metric::new(None, MetricSource::Unavailable),
            duration_us: None,
            measurement_id: None,
        };
        let json = serde_json::to_string(&value).expect("serialize DTO");
        assert!(json.contains("\"value\":null"));
        let decoded: SessionSummary = serde_json::from_str(&json).expect("deserialize DTO");
        assert_eq!(decoded, value);
    }

    #[test]
    fn serde_json_rejects_non_finite_input() {
        let result =
            serde_json::from_str::<Metric<f64>>(r#"{"value":NaN,"source":"locally_estimated"}"#);
        assert!(result.is_err());
    }
}
