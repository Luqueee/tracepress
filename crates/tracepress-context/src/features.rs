//! Bounded per-block features and the opportunity signals derived from them.
//!
//! Every value here is an aggregate over one block: there is never one record per line, per JSON
//! item, or per token. Inspection is bounded by the configured analysis limits, and a feature
//! computed from a view that a bound cut short is marked through [`FeatureCoverage`] rather than
//! presented as a complete count. A feature that cannot be computed is absent, never zero: an
//! empty content has zero lines, while a content with no lines has an unknown maximum line
//! length.
//!
//! Detection is an input, not a dependency: the caller passes the [`DetectionResult`] its
//! detectors already produced, so feature extraction stays a pure function of bytes plus that
//! result.
//!
//! Opportunity signals are signals. Nothing here chooses a compressor, computes a target ratio,
//! or claims a saving; `candidate_estimated_tokens` reports the tokens already estimated for the
//! block a signal points at.

use std::{collections::HashSet, fmt};

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{
    ContextAnalysisLimits, ContextBlockKind, DetectedContentKind, DetectionConfidence,
    DetectionResult,
};

/// Line prefixes and words that mark a failure in log, test, or tool output.
const ERROR_TOKENS: &[&[u8]] = &[
    b"error",
    b"errors",
    b"fatal",
    b"panic",
    b"panicked",
    b"exception",
    b"traceback",
];

/// Words that mark a warning in log, test, or tool output.
const WARNING_TOKENS: &[&[u8]] = &[b"warning", b"warnings", b"warn", b"deprecated"];

/// An exact ratio in `[0.0, 1.0]`.
///
/// The value is stored as parts per million rather than as a float so a ratio is comparable,
/// hashable, and byte-stable across platforms, and so the range invariant holds by construction
/// instead of by convention. A ratio whose denominator is zero does not exist: the feature that
/// would have carried it is absent instead.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct FeatureRatio {
    parts_per_million: u32,
}

impl FeatureRatio {
    /// Parts per million representing the whole.
    pub const PARTS_PER_UNIT: u32 = 1_000_000;
    /// The ratio `0.0`.
    pub const ZERO: Self = Self {
        parts_per_million: 0,
    };
    /// The ratio `1.0`.
    pub const ONE: Self = Self {
        parts_per_million: Self::PARTS_PER_UNIT,
    };

    /// Builds a ratio from a numerator and a denominator, truncating toward zero.
    ///
    /// Returns `None` for a zero denominator, which means the ratio is unknown rather than zero.
    /// A numerator above the denominator is saturated at the whole, so no arithmetic accident can
    /// produce a ratio above `1.0`.
    #[must_use]
    pub fn from_ratio(numerator: u64, denominator: u64) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        let bounded_numerator = numerator.min(denominator);
        let scaled = u128::from(bounded_numerator)
            .checked_mul(u128::from(Self::PARTS_PER_UNIT))?
            .checked_div(u128::from(denominator))?;
        u32::try_from(scaled)
            .ok()
            .map(|parts_per_million| Self { parts_per_million })
    }

    /// Builds a ratio from parts per million, clamping at the whole.
    ///
    /// This is the constructor for declared thresholds, where clamping a compile-time constant is
    /// the intended behaviour.
    #[must_use]
    pub const fn from_parts_per_million_clamped(parts_per_million: u32) -> Self {
        Self {
            parts_per_million: if parts_per_million > Self::PARTS_PER_UNIT {
                Self::PARTS_PER_UNIT
            } else {
                parts_per_million
            },
        }
    }

    /// Builds a ratio from parts per million, rejecting a value above the whole.
    ///
    /// This is the constructor for a transported value, where a remote writer must not be able to
    /// introduce a ratio outside the range.
    #[must_use]
    pub const fn from_parts_per_million(parts_per_million: u32) -> Option<Self> {
        if parts_per_million > Self::PARTS_PER_UNIT {
            return None;
        }
        Some(Self { parts_per_million })
    }

    /// Returns the ratio as parts per million, always in `0..=1_000_000`.
    #[must_use]
    pub const fn parts_per_million(self) -> u32 {
        self.parts_per_million
    }

    /// Returns the ratio as a float, always in `[0.0, 1.0]`.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        f64::from(self.parts_per_million) / f64::from(Self::PARTS_PER_UNIT)
    }
}

impl<'de> Deserialize<'de> for FeatureRatio {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let parts_per_million = u32::deserialize(deserializer)?;
        Self::from_parts_per_million(parts_per_million)
            .ok_or_else(|| de::Error::custom("a feature ratio above one million parts per million"))
    }
}

/// How much of one block the feature pass actually saw.
///
/// Crossing a bound stops that dimension and is recorded here; the aggregates already completed
/// inside the bound are kept rather than discarded.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct FeatureCoverage {
    /// Content bytes admitted to inspection.
    pub inspected_bytes: u64,
    /// Content bytes the inspection bound left out.
    pub skipped_bytes: u64,
    /// The inspected content is a prefix of a longer content.
    ///
    /// Set either because the caller's decode was already truncated or because the inspection
    /// bound cut the content short.
    pub content_truncated: bool,
    /// Lines that took part in the duplicate, unique, and repetition aggregates.
    pub ratio_sampled_lines: u64,
    /// The line aggregates stopped at the distinct-line bound and cover a prefix of the lines.
    pub line_features_bounded: bool,
    /// The JSON aggregates were cut short by the depth bound or by a truncated content.
    pub json_features_bounded: bool,
}

impl FeatureCoverage {
    /// Returns whether every recorded feature covers the whole block.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        !self.content_truncated && !self.line_features_bounded && !self.json_features_bounded
    }
}

/// Bounded aggregate features of one context block.
///
/// The line and JSON aggregates describe the inspected view, which [`Self::coverage`] qualifies.
/// [`Self::complete_line_count`] and [`Self::complete_json_item_count`] exist so a consumer that
/// needs a complete count cannot mistake a bounded one for it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct BlockFeatures {
    /// Bytes the block occupies in the original request.
    pub raw_bytes: u64,
    /// Lines observed in the inspected content, absent where lines are not meaningful.
    pub line_count: Option<u64>,
    /// Longest observed line in bytes, excluding its terminator, absent where no line exists.
    pub max_line_bytes: Option<u64>,
    /// Share of sampled lines that repeat an earlier sampled line.
    pub duplicate_line_ratio: Option<FeatureRatio>,
    /// Share of sampled lines that are distinct.
    pub unique_line_ratio: Option<FeatureRatio>,
    /// Direct children of the top-level JSON value, absent without a JSON detection.
    pub json_item_count: Option<u64>,
    /// Maximum JSON container nesting depth, clamped at the configured depth bound.
    pub json_depth: Option<u32>,
    /// Share of observed lines carrying an error marker.
    pub error_line_density: Option<FeatureRatio>,
    /// Share of observed lines carrying a warning marker.
    pub warning_line_density: Option<FeatureRatio>,
    /// Share of sampled line bytes belonging to a repeated line.
    pub repetition_score: Option<FeatureRatio>,
    /// Content shape the caller's detector recognized, absent where no detector ran.
    pub detected_kind: Option<DetectedContentKind>,
    /// How much of the block the aggregates above cover.
    coverage: FeatureCoverage,
}

impl BlockFeatures {
    /// Distinct lines one block may hold in the duplicate-tracking table.
    ///
    /// The table holds borrowed line slices rather than copies, and the inspected content is
    /// already bounded, so this bound exists to keep the table itself small on content that is
    /// nothing but short distinct lines. Reaching it stops the duplicate, unique, and repetition
    /// aggregates at the lines already sampled and marks them bounded; the samples taken before
    /// the bound are kept.
    pub const MAX_TRACKED_DISTINCT_LINES: usize = 8192;

    /// Returns how much of the block the recorded aggregates cover.
    #[must_use]
    pub const fn coverage(&self) -> FeatureCoverage {
        self.coverage
    }

    /// Returns the line count only when it covers the whole block.
    #[must_use]
    pub const fn complete_line_count(&self) -> Option<u64> {
        if self.coverage.content_truncated {
            return None;
        }
        self.line_count
    }

    /// Returns the JSON item count only when it covers the whole block.
    #[must_use]
    pub const fn complete_json_item_count(&self) -> Option<u64> {
        if self.coverage.json_features_bounded {
            return None;
        }
        self.json_item_count
    }
}

/// Everything the feature pass needs about one block.
///
/// `content` is the decoded or raw payload of the block and may be longer than the inspection
/// bound; the pass takes a shared slice, never mutates it, and inspects at most
/// `limits.max_string_bytes_inspected` of it.
#[allow(
    clippy::exhaustive_structs,
    reason = "the caller constructs this input, so a new required input must break every caller"
)]
#[derive(Clone, Copy)]
pub struct BlockFeatureInput<'content> {
    /// Bytes the block occupies in the original request.
    pub raw_bytes: u64,
    /// Content admitted to inspection.
    pub content: &'content [u8],
    /// The caller's decode of `content` was already truncated.
    pub content_truncated: bool,
    /// Detection result the caller's detectors already produced.
    pub detection: Option<DetectionResult>,
    /// Bounds this analysis observes.
    pub limits: ContextAnalysisLimits,
}

impl fmt::Debug for BlockFeatureInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BlockFeatureInput")
            .field("raw_bytes", &self.raw_bytes)
            .field("content_bytes", &self.content.len())
            .field("content_truncated", &self.content_truncated)
            .field("detection", &self.detection)
            .finish_non_exhaustive()
    }
}

/// Extracts the bounded aggregate features of one block.
///
/// The pass never mutates the content, never allocates in proportion to a declared length, and
/// never records a value per line: it walks the inspected prefix once for line aggregates and
/// once more for JSON aggregates, both bounded by `input.limits`.
#[must_use]
pub fn extract_block_features(input: &BlockFeatureInput<'_>) -> BlockFeatures {
    let bound = input.limits.max_string_bytes_inspected.get();
    let inspected = input.content.get(..bound).unwrap_or(input.content);
    let skipped = input.content.len().saturating_sub(inspected.len());
    let truncated = input.content_truncated || skipped > 0;
    let detected_kind = input.detection.map(|detection| detection.kind);

    let lines = if detected_kind == Some(DetectedContentKind::BinaryLike) {
        None
    } else {
        Some(line_aggregates(inspected, truncated))
    };
    let json = json_aggregates(
        inspected,
        detected_kind,
        JsonView {
            truncated,
            max_depth: input.limits.max_json_depth.get(),
        },
    );

    let coverage = FeatureCoverage {
        inspected_bytes: bytes_of(inspected.len()),
        skipped_bytes: bytes_of(skipped),
        content_truncated: truncated,
        ratio_sampled_lines: lines.map_or(0, |aggregates| aggregates.sampled_lines),
        line_features_bounded: lines.is_some_and(|aggregates| aggregates.bounded),
        json_features_bounded: json.is_some_and(|aggregates| aggregates.bounded),
    };

    BlockFeatures {
        raw_bytes: input.raw_bytes,
        line_count: lines.map(|aggregates| aggregates.line_count),
        max_line_bytes: lines.and_then(LineAggregates::max_line_bytes),
        duplicate_line_ratio: lines.and_then(LineAggregates::duplicate_line_ratio),
        unique_line_ratio: lines.and_then(LineAggregates::unique_line_ratio),
        json_item_count: json.and_then(|aggregates| aggregates.item_count),
        json_depth: json.and_then(|aggregates| aggregates.depth),
        error_line_density: lines.and_then(LineAggregates::error_line_density),
        warning_line_density: lines.and_then(LineAggregates::warning_line_density),
        repetition_score: lines.and_then(LineAggregates::repetition_score),
        detected_kind,
        coverage,
    }
}

/// One structural reason a block might repay a closer look later.
///
/// These are observations, not decisions. No member names a compressor, a target ratio, or a
/// saving, and nothing in this phase acts on one.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OpportunitySignal {
    /// A large tool result.
    LargeToolResult,
    /// A high share of repeated lines.
    HighDuplication,
    /// Many JSON items at a shallow, uniform depth.
    HomogeneousJson,
    /// Log output with a high share of repeated lines.
    RepetitiveLogs,
    /// A large search result.
    LargeSearchResult,
    /// A large test-runner output.
    LargeTestOutput,
    /// A large tool schema.
    LargeToolSchema,
    /// A block whose bytes were already sent earlier in the session.
    RepeatedHistory,
}

impl OpportunitySignal {
    /// Every signal this analysis version derives, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::LargeToolResult,
        Self::HighDuplication,
        Self::HomogeneousJson,
        Self::RepetitiveLogs,
        Self::LargeSearchResult,
        Self::LargeTestOutput,
        Self::LargeToolSchema,
        Self::RepeatedHistory,
    ];
    /// Bytes above which a tool result, search result, or test output counts as large.
    pub const LARGE_BLOCK_BYTES: u64 = 8192;
    /// Bytes above which a tool schema counts as large.
    pub const LARGE_TOOL_SCHEMA_BYTES: u64 = 4096;
    /// Bytes above which a repeated block is worth signalling.
    pub const REPEATED_HISTORY_MIN_BYTES: u64 = 1024;
    /// Sampled lines a block needs before a duplicate ratio means anything.
    pub const HIGH_DUPLICATION_MIN_LINES: u64 = 8;
    /// Duplicate line share at which duplication counts as high.
    pub const HIGH_DUPLICATION_RATIO: FeatureRatio =
        FeatureRatio::from_parts_per_million_clamped(500_000);
    /// Items a JSON block needs before homogeneity means anything.
    pub const HOMOGENEOUS_JSON_MIN_ITEMS: u64 = 8;
    /// Nesting depth above which a JSON block is not treated as uniform.
    pub const HOMOGENEOUS_JSON_MAX_DEPTH: u32 = 6;
    /// Lines a log block needs before repetition means anything.
    pub const REPETITIVE_LOG_MIN_LINES: u64 = 32;
    /// Duplicate line share at which log output counts as repetitive.
    pub const REPETITIVE_LOG_MIN_DUPLICATE_RATIO: FeatureRatio =
        FeatureRatio::from_parts_per_million_clamped(200_000);

    /// Returns the stable wire and durable name of this signal.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::LargeToolResult => "large_tool_result",
            Self::HighDuplication => "high_duplication",
            Self::HomogeneousJson => "homogeneous_json",
            Self::RepetitiveLogs => "repetitive_logs",
            Self::LargeSearchResult => "large_search_result",
            Self::LargeTestOutput => "large_test_output",
            Self::LargeToolSchema => "large_tool_schema",
            Self::RepeatedHistory => "repeated_history",
        }
    }

    const fn bit(self) -> u16 {
        match self {
            Self::LargeToolResult => 1 << 0,
            Self::HighDuplication => 1 << 1,
            Self::HomogeneousJson => 1 << 2,
            Self::RepetitiveLogs => 1 << 3,
            Self::LargeSearchResult => 1 << 4,
            Self::LargeTestOutput => 1 << 5,
            Self::LargeToolSchema => 1 << 6,
            Self::RepeatedHistory => 1 << 7,
        }
    }
}

impl fmt::Display for OpportunitySignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_wire_str())
    }
}

/// The signals derived for one block.
///
/// `candidate_estimated_tokens` is the block's already-estimated token count, recorded only when
/// at least one signal fired and an estimate exists. It is not a saving, and no saving,
/// compressor, or target ratio is derived anywhere in this phase.
#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
pub struct OpportunitySignalSet {
    bits: u16,
    candidate_estimated_tokens: Option<u64>,
}

impl OpportunitySignalSet {
    const ALL_BITS: u16 = OpportunitySignal::LargeToolResult.bit()
        | OpportunitySignal::HighDuplication.bit()
        | OpportunitySignal::HomogeneousJson.bit()
        | OpportunitySignal::RepetitiveLogs.bit()
        | OpportunitySignal::LargeSearchResult.bit()
        | OpportunitySignal::LargeTestOutput.bit()
        | OpportunitySignal::LargeToolSchema.bit()
        | OpportunitySignal::RepeatedHistory.bit();

    /// The empty set.
    pub const EMPTY: Self = Self {
        bits: 0,
        candidate_estimated_tokens: None,
    };

    /// Returns whether the signal fired.
    #[must_use]
    pub const fn contains(self, signal: OpportunitySignal) -> bool {
        self.bits & signal.bit() != 0
    }

    /// Returns whether no signal fired.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

    /// Returns how many signals fired.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.bits.count_ones()
    }

    /// Returns the signals that fired, in declaration order.
    pub fn iter(self) -> impl Iterator<Item = OpportunitySignal> {
        OpportunitySignal::ALL
            .into_iter()
            .filter(move |signal| self.contains(*signal))
    }

    /// Returns the estimated tokens of the block these signals point at.
    #[must_use]
    pub const fn candidate_estimated_tokens(self) -> Option<u64> {
        self.candidate_estimated_tokens
    }
}

impl fmt::Debug for OpportunitySignalSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpportunitySignalSet")
            .field("signals", &DebugSignals(*self))
            .field(
                "candidate_estimated_tokens",
                &self.candidate_estimated_tokens,
            )
            .finish_non_exhaustive()
    }
}

struct DebugSignals(OpportunitySignalSet);

impl fmt::Debug for DebugSignals {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.0.iter()).finish()
    }
}

impl<'de> Deserialize<'de> for OpportunitySignalSet {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let transported = TransportedSignalSet::deserialize(deserializer)?;
        if transported.bits & !Self::ALL_BITS != 0 {
            return Err(de::Error::custom(
                "an opportunity signal this analysis version does not define",
            ));
        }
        if transported.bits == 0 && transported.candidate_estimated_tokens.is_some() {
            return Err(de::Error::custom(
                "candidate estimated tokens without any opportunity signal",
            ));
        }
        Ok(Self {
            bits: transported.bits,
            candidate_estimated_tokens: transported.candidate_estimated_tokens,
        })
    }
}

#[derive(Deserialize)]
struct TransportedSignalSet {
    bits: u16,
    candidate_estimated_tokens: Option<u64>,
}

/// Everything the signal derivation needs about one block.
#[allow(
    clippy::exhaustive_structs,
    reason = "the caller constructs this input, so a new required input must break every caller"
)]
#[derive(Clone, Copy, Debug)]
pub struct OpportunitySignalInput<'features> {
    /// Canonical kind of the block.
    pub kind: ContextBlockKind,
    /// Aggregate features already extracted for the block.
    pub features: &'features BlockFeatures,
    /// Detection result the caller's detectors already produced.
    pub detection: Option<DetectionResult>,
    /// Tokens already estimated for the block, absent where no estimate applies.
    pub estimated_tokens: Option<u64>,
    /// The block's bytes were already observed earlier in the same session.
    ///
    /// Repetition across snapshots is a delta-stage observation; [`OpportunitySignal::
    /// RepeatedHistory`] fires only when the caller passes it.
    pub repeated_in_session: bool,
}

/// Derives the opportunity signals of one block from its features.
///
/// The derivation is pure and total: it reads the aggregates and the detection result, fires only
/// the conditions documented on [`OpportunitySignal`], and never fires on a detection below
/// [`DetectionConfidence::Medium`], because a detector that cannot commit must not become a
/// signal.
#[must_use]
pub fn derive_opportunity_signals(input: &OpportunitySignalInput<'_>) -> OpportunitySignalSet {
    let mut bits = 0_u16;
    for (signal, fired) in [
        (OpportunitySignal::LargeToolResult, large_tool_result(input)),
        (OpportunitySignal::HighDuplication, high_duplication(input)),
        (OpportunitySignal::HomogeneousJson, homogeneous_json(input)),
        (OpportunitySignal::RepetitiveLogs, repetitive_logs(input)),
        (
            OpportunitySignal::LargeSearchResult,
            large_detected(input, DetectedContentKind::SearchResults),
        ),
        (
            OpportunitySignal::LargeTestOutput,
            large_detected(input, DetectedContentKind::TestResults),
        ),
        (OpportunitySignal::LargeToolSchema, large_tool_schema(input)),
        (OpportunitySignal::RepeatedHistory, repeated_history(input)),
    ] {
        if fired {
            bits |= signal.bit();
        }
    }
    OpportunitySignalSet {
        bits,
        candidate_estimated_tokens: if bits == 0 {
            None
        } else {
            input.estimated_tokens
        },
    }
}

fn large_tool_result(input: &OpportunitySignalInput<'_>) -> bool {
    input.kind == ContextBlockKind::ToolResult
        && input.features.raw_bytes >= OpportunitySignal::LARGE_BLOCK_BYTES
}

fn large_tool_schema(input: &OpportunitySignalInput<'_>) -> bool {
    input.kind == ContextBlockKind::ToolDefinition
        && input.features.raw_bytes >= OpportunitySignal::LARGE_TOOL_SCHEMA_BYTES
}

const fn repeated_history(input: &OpportunitySignalInput<'_>) -> bool {
    input.repeated_in_session
        && input.features.raw_bytes >= OpportunitySignal::REPEATED_HISTORY_MIN_BYTES
}

fn high_duplication(input: &OpportunitySignalInput<'_>) -> bool {
    input.features.coverage.ratio_sampled_lines >= OpportunitySignal::HIGH_DUPLICATION_MIN_LINES
        && input
            .features
            .duplicate_line_ratio
            .is_some_and(|ratio| ratio >= OpportunitySignal::HIGH_DUPLICATION_RATIO)
}

fn homogeneous_json(input: &OpportunitySignalInput<'_>) -> bool {
    let json_detected =
        detected(input, DetectedContentKind::Json) || detected(input, DetectedContentKind::Ndjson);
    json_detected
        && input
            .features
            .json_item_count
            .is_some_and(|count| count >= OpportunitySignal::HOMOGENEOUS_JSON_MIN_ITEMS)
        && input
            .features
            .json_depth
            .is_some_and(|depth| depth <= OpportunitySignal::HOMOGENEOUS_JSON_MAX_DEPTH)
}

fn repetitive_logs(input: &OpportunitySignalInput<'_>) -> bool {
    detected(input, DetectedContentKind::Log)
        && input.features.coverage.ratio_sampled_lines
            >= OpportunitySignal::REPETITIVE_LOG_MIN_LINES
        && input
            .features
            .duplicate_line_ratio
            .is_some_and(|ratio| ratio >= OpportunitySignal::REPETITIVE_LOG_MIN_DUPLICATE_RATIO)
}

fn large_detected(input: &OpportunitySignalInput<'_>, kind: DetectedContentKind) -> bool {
    detected(input, kind) && input.features.raw_bytes >= OpportunitySignal::LARGE_BLOCK_BYTES
}

fn detected(input: &OpportunitySignalInput<'_>, kind: DetectedContentKind) -> bool {
    input.detection.is_some_and(|detection| {
        detection.kind == kind && detection.confidence >= DetectionConfidence::Medium
    })
}

/// Aggregates of the lines of one inspected content.
#[derive(Clone, Copy, Default)]
struct LineAggregates {
    line_count: u64,
    longest_line_bytes: u64,
    distinct_lines: u64,
    duplicate_lines: u64,
    error_lines: u64,
    warning_lines: u64,
    sampled_lines: u64,
    sampled_bytes: u64,
    repeated_bytes: u64,
    bounded: bool,
}

impl LineAggregates {
    fn max_line_bytes(self) -> Option<u64> {
        (self.line_count > 0).then_some(self.longest_line_bytes)
    }

    fn duplicate_line_ratio(self) -> Option<FeatureRatio> {
        FeatureRatio::from_ratio(self.duplicate_lines, self.sampled_lines)
    }

    fn unique_line_ratio(self) -> Option<FeatureRatio> {
        FeatureRatio::from_ratio(self.distinct_lines, self.sampled_lines)
    }

    fn repetition_score(self) -> Option<FeatureRatio> {
        FeatureRatio::from_ratio(self.repeated_bytes, self.sampled_bytes)
    }

    fn error_line_density(self) -> Option<FeatureRatio> {
        FeatureRatio::from_ratio(self.error_lines, self.line_count)
    }

    fn warning_line_density(self) -> Option<FeatureRatio> {
        FeatureRatio::from_ratio(self.warning_lines, self.line_count)
    }
}

fn line_aggregates(inspected: &[u8], truncated: bool) -> LineAggregates {
    let mut aggregates = LineAggregates::default();
    let mut seen: HashSet<&[u8]> = HashSet::new();
    for line in complete_lines(inspected, truncated) {
        let line_bytes = bytes_of(line.len());
        aggregates.line_count = aggregates.line_count.saturating_add(1);
        aggregates.longest_line_bytes = aggregates.longest_line_bytes.max(line_bytes);
        if contains_any_token(line, ERROR_TOKENS) {
            aggregates.error_lines = aggregates.error_lines.saturating_add(1);
        }
        if contains_any_token(line, WARNING_TOKENS) {
            aggregates.warning_lines = aggregates.warning_lines.saturating_add(1);
        }
        if aggregates.bounded {
            continue;
        }
        if seen.len() >= BlockFeatures::MAX_TRACKED_DISTINCT_LINES && !seen.contains(line) {
            aggregates.bounded = true;
            continue;
        }
        aggregates.sampled_lines = aggregates.sampled_lines.saturating_add(1);
        aggregates.sampled_bytes = aggregates.sampled_bytes.saturating_add(line_bytes);
        if seen.insert(line) {
            aggregates.distinct_lines = aggregates.distinct_lines.saturating_add(1);
        } else {
            aggregates.duplicate_lines = aggregates.duplicate_lines.saturating_add(1);
            aggregates.repeated_bytes = aggregates.repeated_bytes.saturating_add(line_bytes);
        }
    }
    aggregates
}

/// Yields the lines of `inspected` that were observed whole.
///
/// A trailing segment without a terminator is a line only when the content was not truncated: an
/// unterminated tail of a truncated view is an unfinished line, and counting it would present a
/// fragment as a line.
fn complete_lines(inspected: &[u8], truncated: bool) -> impl Iterator<Item = &[u8]> {
    let mut segments = inspected.split(|byte| *byte == b'\n').peekable();
    std::iter::from_fn(move || {
        let segment = segments.next()?;
        if segments.peek().is_none() && (segment.is_empty() || truncated) {
            return None;
        }
        Some(strip_carriage_return(segment))
    })
}

const fn strip_carriage_return(segment: &[u8]) -> &[u8] {
    match segment.split_last() {
        Some((&b'\r', head)) => head,
        _ => segment,
    }
}

fn contains_any_token(line: &[u8], tokens: &[&[u8]]) -> bool {
    tokens
        .iter()
        .any(|token| contains_token_at_word_boundary(line, token))
}

/// Returns whether `line` contains `token` as a whole ASCII word, ignoring case.
///
/// Requiring a word boundary keeps a density honest: `terrorized` is not an error line, and
/// `warnings` is matched by its own token rather than by a prefix of another word.
fn contains_token_at_word_boundary(line: &[u8], token: &[u8]) -> bool {
    if token.is_empty() || line.len() < token.len() {
        return false;
    }
    let last_start = line.len().saturating_sub(token.len());
    (0..=last_start).any(|start| {
        let end = start.saturating_add(token.len());
        line.get(start..end)
            .is_some_and(|window| window.eq_ignore_ascii_case(token))
            && !is_word_byte_at(line, start.checked_sub(1))
            && !is_word_byte_at(line, Some(end))
    })
}

fn is_word_byte_at(line: &[u8], index: Option<usize>) -> bool {
    index
        .and_then(|index| line.get(index))
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

/// Aggregates of the JSON shape of one inspected content.
#[derive(Clone, Copy, Default)]
struct JsonAggregates {
    item_count: Option<u64>,
    depth: Option<u32>,
    bounded: bool,
}

/// The inspected view and the depth bound one JSON pass observes.
#[derive(Clone, Copy)]
struct JsonView {
    truncated: bool,
    max_depth: u64,
}

fn json_aggregates(
    inspected: &[u8],
    detected_kind: Option<DetectedContentKind>,
    view: JsonView,
) -> Option<JsonAggregates> {
    let mut aggregates = match detected_kind? {
        DetectedContentKind::Json => document_aggregates(inspected, view.max_depth),
        DetectedContentKind::Ndjson => newline_delimited_aggregates(inspected, view),
        _ => return None,
    };
    aggregates.bounded = aggregates.bounded || view.truncated;
    Some(aggregates)
}

fn document_aggregates(inspected: &[u8], max_depth: u64) -> JsonAggregates {
    let scan = scan_json(inspected, max_depth);
    let item_count = if scan.is_container {
        Some(if scan.container_populated {
            scan.top_level_separators.saturating_add(1)
        } else {
            0
        })
    } else if scan.saw_value {
        Some(1)
    } else {
        None
    };
    JsonAggregates {
        item_count,
        depth: scan.reported_depth(max_depth),
        bounded: scan.bounded,
    }
}

fn newline_delimited_aggregates(inspected: &[u8], view: JsonView) -> JsonAggregates {
    let max_depth = view.max_depth;
    let mut item_count = 0_u64;
    let mut depth = 0_u32;
    let mut bounded = false;
    for line in complete_lines(inspected, view.truncated) {
        let trimmed = line.trim_ascii_start();
        if !matches!(trimmed.first(), Some(&b'{' | &b'[')) {
            continue;
        }
        item_count = item_count.saturating_add(1);
        let scan = scan_json(line, max_depth);
        depth = depth.max(scan.reported_depth(max_depth).unwrap_or(0));
        bounded = bounded || scan.bounded;
    }
    JsonAggregates {
        item_count: Some(item_count),
        depth: (item_count > 0).then_some(depth),
        bounded,
    }
}

/// A single bounded pass over JSON-shaped bytes.
///
/// The pass is iterative and carries only counters, so no input nesting can recurse it and no
/// declared length can size an allocation. Depth beyond the configured bound is reported as
/// bounded rather than descended.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent structural observation of one bounded pass"
)]
#[derive(Clone, Copy, Default)]
struct JsonScan {
    max_depth: u32,
    top_level_separators: u64,
    is_container: bool,
    container_populated: bool,
    saw_value: bool,
    bounded: bool,
}

impl JsonScan {
    fn reported_depth(self, max_depth: u64) -> Option<u32> {
        if self.is_container {
            let bound = u32::try_from(max_depth).unwrap_or(u32::MAX);
            return Some(self.max_depth.min(bound));
        }
        self.saw_value.then_some(0)
    }
}

fn scan_json(bytes: &[u8], max_depth: u64) -> JsonScan {
    let mut scan = JsonScan::default();
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;
    for byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => {
                mark_value(&mut scan, depth);
                in_string = true;
            }
            b'{' | b'[' => {
                mark_value(&mut scan, depth);
                if depth == 0 {
                    scan.is_container = true;
                }
                depth = depth.saturating_add(1);
                scan.max_depth = scan.max_depth.max(depth);
                if u64::from(depth) > max_depth {
                    scan.bounded = true;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 1 => {
                scan.top_level_separators = scan.top_level_separators.saturating_add(1);
            }
            other if !other.is_ascii_whitespace() => mark_value(&mut scan, depth),
            _ => {}
        }
    }
    scan
}

/// Records that a value byte was seen, and that the top-level container holds a member.
///
/// A closing delimiter is deliberately not a value byte: counting it would make an empty
/// container look populated and report one item where there are none.
const fn mark_value(scan: &mut JsonScan, depth: u32) {
    scan.saw_value = true;
    if depth == 1 {
        scan.container_populated = true;
    }
}

fn bytes_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::fmt::{self, Write as _};

    use super::{
        BlockFeatureInput, BlockFeatures, ContextAnalysisLimits, ContextBlockKind,
        DetectedContentKind, DetectionConfidence, DetectionResult, FeatureRatio, OpportunitySignal,
        OpportunitySignalInput, OpportunitySignalSet, derive_opportunity_signals,
        extract_block_features,
    };
    use crate::{ContextAnalysisLimitValues, ContextAnalysisLimitsError};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn limits() -> Result<ContextAnalysisLimits, ContextAnalysisLimitsError> {
        limits_with_inspection_bound(ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED)
    }

    fn limits_with_inspection_bound(
        bound: u64,
    ) -> Result<ContextAnalysisLimits, ContextAnalysisLimitsError> {
        ContextAnalysisLimits::new(ContextAnalysisLimitValues {
            max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
            max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
            max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
            max_string_bytes_inspected: bound,
            max_analysis_work_units: 2_000,
            max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
            max_batches: ContextAnalysisLimits::MAX_BATCHES,
        })
    }

    fn bytes_of(value: usize) -> u64 {
        u64::try_from(value).unwrap_or(u64::MAX)
    }

    /// Builds `count` distinct lines, each `prefix` followed by its index.
    fn numbered_lines(count: u32, prefix: &str) -> Result<String, fmt::Error> {
        (0..count).try_fold(String::new(), |mut text, index| {
            writeln!(text, "{prefix}{index}")?;
            Ok(text)
        })
    }

    fn features_of(content: &[u8]) -> Result<BlockFeatures, ContextAnalysisLimitsError> {
        Ok(extract_block_features(&BlockFeatureInput {
            raw_bytes: bytes_of(content.len()),
            content,
            content_truncated: false,
            detection: None,
            limits: limits()?,
        }))
    }

    fn detected_features(
        content: &[u8],
        detection: DetectionResult,
    ) -> Result<BlockFeatures, ContextAnalysisLimitsError> {
        Ok(extract_block_features(&BlockFeatureInput {
            raw_bytes: bytes_of(content.len()),
            content,
            content_truncated: false,
            detection: Some(detection),
            limits: limits()?,
        }))
    }

    fn detection(kind: DetectedContentKind, confidence: DetectionConfidence) -> DetectionResult {
        DetectionResult {
            kind,
            confidence,
            detector_version: 1,
        }
    }

    fn ratio(numerator: u64, denominator: u64) -> Option<FeatureRatio> {
        FeatureRatio::from_ratio(numerator, denominator)
    }

    fn signals_of(
        kind: ContextBlockKind,
        features: &BlockFeatures,
        detection: Option<DetectionResult>,
    ) -> OpportunitySignalSet {
        derive_opportunity_signals(&OpportunitySignalInput {
            kind,
            features,
            detection,
            estimated_tokens: Some(4096),
            repeated_in_session: false,
        })
    }

    fn assert_ratios_in_range(features: &BlockFeatures) {
        for ratio in [
            features.duplicate_line_ratio,
            features.unique_line_ratio,
            features.error_line_density,
            features.warning_line_density,
            features.repetition_score,
        ]
        .into_iter()
        .flatten()
        {
            assert!(
                (0.0..=1.0).contains(&ratio.as_f64()),
                "a ratio must stay inside its range"
            );
            assert!(ratio <= FeatureRatio::ONE, "a ratio must not exceed one");
        }
    }

    #[test]
    fn duplicate_heavy_content_reports_a_high_duplicate_ratio() -> TestResult {
        let content = "same line\n".repeat(64);

        let features = features_of(content.as_bytes())?;

        assert_eq!(features.line_count, Some(64));
        assert_eq!(features.duplicate_line_ratio, ratio(63, 64));
        assert_eq!(features.unique_line_ratio, ratio(1, 64));
        assert_eq!(features.repetition_score, ratio(63, 64));
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn unique_heavy_content_reports_a_low_duplicate_ratio() -> TestResult {
        let content = numbered_lines(64, "distinct line ")?;

        let features = features_of(content.as_bytes())?;

        assert_eq!(features.line_count, Some(64));
        assert_eq!(features.duplicate_line_ratio, Some(FeatureRatio::ZERO));
        assert_eq!(features.unique_line_ratio, Some(FeatureRatio::ONE));
        assert_eq!(features.repetition_score, Some(FeatureRatio::ZERO));
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn empty_content_reports_no_lines_and_no_ratios() -> TestResult {
        let features = features_of(b"")?;

        assert_eq!(features.line_count, Some(0));
        assert_eq!(features.max_line_bytes, None);
        assert_eq!(features.duplicate_line_ratio, None);
        assert_eq!(features.unique_line_ratio, None);
        assert_eq!(features.error_line_density, None);
        assert_eq!(features.repetition_score, None);
        assert!(features.coverage().is_complete());
        Ok(())
    }

    #[test]
    fn a_single_line_without_a_terminator_is_one_complete_line() -> TestResult {
        let features = features_of(b"only line")?;

        assert_eq!(features.line_count, Some(1));
        assert_eq!(features.complete_line_count(), Some(1));
        assert_eq!(features.max_line_bytes, Some(9));
        assert_eq!(features.unique_line_ratio, Some(FeatureRatio::ONE));
        assert_eq!(features.duplicate_line_ratio, Some(FeatureRatio::ZERO));
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn a_trailing_newline_does_not_add_an_empty_line() -> TestResult {
        let terminated = features_of(b"a\nb\n")?;
        let unterminated = features_of(b"a\nb")?;

        assert_eq!(terminated.line_count, Some(2));
        assert_eq!(unterminated.line_count, Some(2));
        assert_eq!(terminated.unique_line_ratio, unterminated.unique_line_ratio);
        Ok(())
    }

    #[test]
    fn carriage_returns_do_not_make_identical_lines_distinct() -> TestResult {
        let features = features_of(b"line\r\nline\r\n")?;

        assert_eq!(features.line_count, Some(2));
        assert_eq!(features.duplicate_line_ratio, ratio(1, 2));
        assert_eq!(features.max_line_bytes, Some(4));
        Ok(())
    }

    #[test]
    fn a_bounded_inspection_is_marked_and_claims_no_complete_count() -> TestResult {
        let content = "line\n".repeat(64);

        let features = extract_block_features(&BlockFeatureInput {
            raw_bytes: bytes_of(content.len()),
            content: content.as_bytes(),
            content_truncated: false,
            detection: None,
            limits: limits_with_inspection_bound(20)?,
        });

        let coverage = features.coverage();
        assert!(coverage.content_truncated, "the bound must be recorded");
        assert!(!coverage.is_complete());
        assert_eq!(coverage.inspected_bytes, 20);
        assert_eq!(
            coverage.skipped_bytes,
            bytes_of(content.len()).saturating_sub(20)
        );
        assert_eq!(features.line_count, Some(4));
        assert_eq!(
            features.complete_line_count(),
            None,
            "a bounded view must not claim a complete count"
        );
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn a_truncated_tail_without_a_terminator_is_not_counted_as_a_line() -> TestResult {
        let features = extract_block_features(&BlockFeatureInput {
            raw_bytes: 32,
            content: b"complete\nincomple",
            content_truncated: true,
            detection: None,
            limits: limits()?,
        });

        assert_eq!(features.line_count, Some(1));
        assert_eq!(features.max_line_bytes, Some(8));
        assert!(features.coverage().content_truncated);
        Ok(())
    }

    #[test]
    fn binary_like_content_reports_absent_line_features_rather_than_zero() -> TestResult {
        let features = detected_features(
            &[0x00, 0xff, 0x00, 0xff],
            detection(DetectedContentKind::BinaryLike, DetectionConfidence::High),
        )?;

        assert_eq!(features.line_count, None);
        assert_eq!(features.max_line_bytes, None);
        assert_eq!(features.duplicate_line_ratio, None);
        assert_eq!(features.repetition_score, None);
        assert_eq!(features.raw_bytes, 4);
        Ok(())
    }

    #[test]
    fn json_features_are_absent_without_a_json_detection() -> TestResult {
        let features = features_of(b"{\"a\": [1, 2, 3]}")?;

        assert_eq!(features.json_item_count, None);
        assert_eq!(features.json_depth, None);
        assert_eq!(features.detected_kind, None);
        Ok(())
    }

    #[test]
    fn a_json_document_reports_its_top_level_items_and_depth() -> TestResult {
        let features = detected_features(
            b"{\"a\": [1, 2, 3], \"b\": {\"c\": 1}}",
            detection(DetectedContentKind::Json, DetectionConfidence::High),
        )?;

        assert_eq!(features.json_item_count, Some(2));
        assert_eq!(features.complete_json_item_count(), Some(2));
        assert_eq!(features.json_depth, Some(2));
        Ok(())
    }

    #[test]
    fn an_empty_json_container_reports_zero_items() -> TestResult {
        let features = detected_features(
            b"[]",
            detection(DetectedContentKind::Json, DetectionConfidence::High),
        )?;

        assert_eq!(features.json_item_count, Some(0));
        assert_eq!(features.json_depth, Some(1));
        Ok(())
    }

    #[test]
    fn a_json_separator_inside_a_string_is_not_an_item_boundary() -> TestResult {
        let features = detected_features(
            b"[\"a,b\", \"c\"]",
            detection(DetectedContentKind::Json, DetectionConfidence::High),
        )?;

        assert_eq!(features.json_item_count, Some(2));
        Ok(())
    }

    #[test]
    fn newline_delimited_json_counts_one_item_per_complete_line() -> TestResult {
        let features = detected_features(
            b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n",
            detection(DetectedContentKind::Ndjson, DetectionConfidence::High),
        )?;

        assert_eq!(features.json_item_count, Some(3));
        assert_eq!(features.json_depth, Some(1));
        assert_eq!(features.line_count, Some(3));
        Ok(())
    }

    #[test]
    fn a_truncated_json_view_does_not_claim_a_complete_item_count() -> TestResult {
        let features = extract_block_features(&BlockFeatureInput {
            raw_bytes: 64,
            content: b"[1, 2, 3, 4, 5, 6, 7, 8]",
            content_truncated: true,
            detection: Some(detection(
                DetectedContentKind::Json,
                DetectionConfidence::High,
            )),
            limits: limits()?,
        });

        assert!(features.json_item_count.is_some());
        assert_eq!(features.complete_json_item_count(), None);
        assert!(features.coverage().json_features_bounded);
        Ok(())
    }

    #[test]
    fn error_and_warning_density_count_only_real_marker_lines() -> TestResult {
        let features = features_of(
            b"ERROR: boom\nwarning: careful\nall good\nthe terrorized parser is fine\n",
        )?;

        assert_eq!(features.line_count, Some(4));
        assert_eq!(features.error_line_density, ratio(1, 4));
        assert_eq!(features.warning_line_density, ratio(1, 4));
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn content_without_markers_reports_zero_density_rather_than_absence() -> TestResult {
        let features = features_of(b"all quiet\nstill quiet\n")?;

        assert_eq!(features.error_line_density, Some(FeatureRatio::ZERO));
        assert_eq!(features.warning_line_density, Some(FeatureRatio::ZERO));
        Ok(())
    }

    #[test]
    fn one_enormous_line_stays_bounded_and_reports_no_complete_line() -> TestResult {
        let content = vec![b'a'; 4 * 1024 * 1024];

        let features = extract_block_features(&BlockFeatureInput {
            raw_bytes: bytes_of(content.len()),
            content: &content,
            content_truncated: false,
            detection: None,
            limits: limits()?,
        });

        let coverage = features.coverage();
        assert_eq!(
            coverage.inspected_bytes,
            ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED
        );
        assert!(coverage.skipped_bytes > 0);
        assert!(coverage.content_truncated);
        assert_eq!(features.line_count, Some(0));
        assert_eq!(features.complete_line_count(), None);
        assert_eq!(features.duplicate_line_ratio, None);
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn millions_of_identical_tiny_lines_stay_bounded_and_in_range() -> TestResult {
        let content = "x\n".repeat(2_000_000);

        let features = extract_block_features(&BlockFeatureInput {
            raw_bytes: bytes_of(content.len()),
            content: content.as_bytes(),
            content_truncated: false,
            detection: None,
            limits: limits()?,
        });

        let inspected = ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED;
        let coverage = features.coverage();
        assert_eq!(coverage.inspected_bytes, inspected);
        assert!(coverage.content_truncated);
        assert_eq!(features.line_count, Some(inspected / 2));
        assert_eq!(features.max_line_bytes, Some(1));
        assert_eq!(features.unique_line_ratio, ratio(1, inspected / 2));
        assert!(!coverage.line_features_bounded);
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn many_distinct_tiny_lines_bound_the_ratio_sample_without_discarding_it() -> TestResult {
        let content = numbered_lines(200_000, "")?;

        let features = extract_block_features(&BlockFeatureInput {
            raw_bytes: bytes_of(content.len()),
            content: content.as_bytes(),
            content_truncated: false,
            detection: None,
            limits: limits()?,
        });

        let coverage = features.coverage();
        assert!(
            coverage.line_features_bounded,
            "distinct lines beyond the tracking bound must mark the line aggregates bounded"
        );
        assert_eq!(
            coverage.ratio_sampled_lines,
            bytes_of(BlockFeatures::MAX_TRACKED_DISTINCT_LINES),
            "the samples taken before the bound must be kept"
        );
        assert!(
            features
                .line_count
                .is_some_and(|count| count > coverage.ratio_sampled_lines)
        );
        assert_eq!(features.unique_line_ratio, Some(FeatureRatio::ONE));
        assert_ratios_in_range(&features);
        Ok(())
    }

    #[test]
    fn a_large_tool_result_signals_only_for_a_tool_result() -> TestResult {
        let content = "line of tool output\n".repeat(600);
        let features = features_of(content.as_bytes())?;

        let tool_result = signals_of(ContextBlockKind::ToolResult, &features, None);
        let message = signals_of(ContextBlockKind::Message, &features, None);

        assert!(tool_result.contains(OpportunitySignal::LargeToolResult));
        assert!(!message.contains(OpportunitySignal::LargeToolResult));
        Ok(())
    }

    #[test]
    fn a_small_tool_result_does_not_signal() -> TestResult {
        let features = features_of(b"tiny\n")?;

        let signals = signals_of(ContextBlockKind::ToolResult, &features, None);

        assert!(!signals.contains(OpportunitySignal::LargeToolResult));
        assert!(signals.is_empty());
        assert_eq!(signals.candidate_estimated_tokens(), None);
        Ok(())
    }

    #[test]
    fn high_duplication_needs_both_enough_lines_and_a_high_ratio() -> TestResult {
        let duplicated = features_of("same\n".repeat(64).as_bytes())?;
        let few_lines = features_of(b"same\nsame\n")?;

        assert!(
            signals_of(ContextBlockKind::Text, &duplicated, None)
                .contains(OpportunitySignal::HighDuplication)
        );
        assert!(
            !signals_of(ContextBlockKind::Text, &few_lines, None)
                .contains(OpportunitySignal::HighDuplication)
        );
        Ok(())
    }

    #[test]
    fn a_low_confidence_detection_never_fires_a_detector_signal() -> TestResult {
        let content = "2026-01-01T00:00:00Z INFO same event\n".repeat(64);
        let abstained = detection(DetectedContentKind::Log, DetectionConfidence::Low);
        let features = detected_features(content.as_bytes(), abstained)?;

        let signals = signals_of(ContextBlockKind::ToolResult, &features, Some(abstained));

        assert!(!signals.contains(OpportunitySignal::RepetitiveLogs));
        Ok(())
    }

    #[test]
    fn repetitive_logs_fire_on_a_confident_detection_with_repeated_lines() -> TestResult {
        let content = "2026-01-01T00:00:00Z INFO same event\n".repeat(64);
        let committed = detection(DetectedContentKind::Log, DetectionConfidence::Medium);
        let features = detected_features(content.as_bytes(), committed)?;

        let signals = signals_of(ContextBlockKind::ToolResult, &features, Some(committed));

        assert!(signals.contains(OpportunitySignal::RepetitiveLogs));
        assert_eq!(signals.candidate_estimated_tokens(), Some(4096));
        Ok(())
    }

    #[test]
    fn repetitive_logs_need_repeated_lines() -> TestResult {
        let content = numbered_lines(64, "2026-01-01T00:00:00Z INFO event ")?;
        let committed = detection(DetectedContentKind::Log, DetectionConfidence::Medium);
        let features = detected_features(content.as_bytes(), committed)?;

        let signals = signals_of(ContextBlockKind::ToolResult, &features, Some(committed));

        assert!(!signals.contains(OpportunitySignal::RepetitiveLogs));
        Ok(())
    }

    #[test]
    fn homogeneous_json_needs_enough_items_at_a_shallow_depth() -> TestResult {
        let committed = detection(DetectedContentKind::Json, DetectionConfidence::High);
        let flat = detected_features(b"[1, 2, 3, 4, 5, 6, 7, 8, 9]", committed)?;
        let small = detected_features(b"[1, 2]", committed)?;

        assert!(
            signals_of(ContextBlockKind::ToolResult, &flat, Some(committed))
                .contains(OpportunitySignal::HomogeneousJson)
        );
        assert!(
            !signals_of(ContextBlockKind::ToolResult, &small, Some(committed))
                .contains(OpportunitySignal::HomogeneousJson)
        );
        Ok(())
    }

    #[test]
    fn a_large_tool_schema_signals_only_for_a_tool_definition() -> TestResult {
        let content = "x".repeat(5000);
        let features = features_of(content.as_bytes())?;

        assert!(
            signals_of(ContextBlockKind::ToolDefinition, &features, None)
                .contains(OpportunitySignal::LargeToolSchema)
        );
        assert!(
            !signals_of(ContextBlockKind::ToolCall, &features, None)
                .contains(OpportunitySignal::LargeToolSchema)
        );
        Ok(())
    }

    #[test]
    fn repeated_history_fires_only_when_the_caller_observed_the_repetition() -> TestResult {
        let content = "y".repeat(2048);
        let features = features_of(content.as_bytes())?;
        let mut input = OpportunitySignalInput {
            kind: ContextBlockKind::AssistantHistory,
            features: &features,
            detection: None,
            estimated_tokens: Some(512),
            repeated_in_session: false,
        };

        assert!(!derive_opportunity_signals(&input).contains(OpportunitySignal::RepeatedHistory));

        input.repeated_in_session = true;
        let signals = derive_opportunity_signals(&input);

        assert!(signals.contains(OpportunitySignal::RepeatedHistory));
        assert_eq!(signals.candidate_estimated_tokens(), Some(512));
        Ok(())
    }

    #[test]
    fn candidate_tokens_are_absent_without_an_estimate() -> TestResult {
        let content = "line of tool output\n".repeat(600);
        let features = features_of(content.as_bytes())?;

        let signals = derive_opportunity_signals(&OpportunitySignalInput {
            kind: ContextBlockKind::ToolResult,
            features: &features,
            detection: None,
            estimated_tokens: None,
            repeated_in_session: false,
        });

        assert!(!signals.is_empty());
        assert_eq!(signals.candidate_estimated_tokens(), None);
        Ok(())
    }

    #[test]
    fn a_signal_set_iterates_exactly_the_signals_that_fired() -> TestResult {
        let content = "same tool line\n".repeat(1200);
        let features = features_of(content.as_bytes())?;
        let signals = signals_of(ContextBlockKind::ToolResult, &features, None);

        let fired: Vec<_> = signals.iter().collect();

        assert_eq!(
            fired,
            vec![
                OpportunitySignal::LargeToolResult,
                OpportunitySignal::HighDuplication
            ]
        );
        assert_eq!(signals.len(), 2);
        Ok(())
    }

    #[test]
    fn a_ratio_cannot_be_built_without_a_denominator() {
        assert_eq!(FeatureRatio::from_ratio(3, 0), None);
        assert_eq!(FeatureRatio::from_ratio(9, 3), Some(FeatureRatio::ONE));
        assert_eq!(FeatureRatio::from_parts_per_million(1_000_001), None);
    }

    #[test]
    fn a_transported_ratio_above_the_whole_is_rejected() {
        let rejected = serde_json::from_str::<FeatureRatio>("1000001");

        assert!(
            rejected.is_err(),
            "an out-of-range ratio must not deserialize"
        );
        assert_eq!(
            serde_json::from_str::<FeatureRatio>("250000").ok(),
            ratio(1, 4)
        );
    }

    #[test]
    fn a_transported_signal_set_rejects_an_undefined_signal() {
        let undefined = serde_json::from_str::<OpportunitySignalSet>(
            "{\"bits\":4096,\"candidate_estimated_tokens\":null}",
        );
        let orphan_tokens = serde_json::from_str::<OpportunitySignalSet>(
            "{\"bits\":0,\"candidate_estimated_tokens\":10}",
        );

        assert!(undefined.is_err());
        assert!(orphan_tokens.is_err());
    }

    #[test]
    fn a_signal_set_survives_the_wire() -> TestResult {
        let content = "same tool line\n".repeat(1200);
        let features = features_of(content.as_bytes())?;
        let signals = signals_of(ContextBlockKind::ToolResult, &features, None);

        let decoded =
            serde_json::from_str::<OpportunitySignalSet>(&serde_json::to_string(&signals)?)?;

        assert_eq!(decoded, signals);
        Ok(())
    }

    #[test]
    fn features_survive_the_wire() -> TestResult {
        let features = detected_features(
            b"{\"a\":1}\n{\"a\":1}\n",
            detection(DetectedContentKind::Ndjson, DetectionConfidence::High),
        )?;

        let decoded = serde_json::from_str::<BlockFeatures>(&serde_json::to_string(&features)?)?;

        assert_eq!(decoded, features);
        Ok(())
    }

    #[test]
    fn debug_output_carries_counts_and_never_content() -> TestResult {
        let content = "secret prompt text\nsecret prompt text\n";
        let features = features_of(content.as_bytes())?;
        let input = BlockFeatureInput {
            raw_bytes: bytes_of(content.len()),
            content: content.as_bytes(),
            content_truncated: false,
            detection: None,
            limits: limits()?,
        };
        let signals = signals_of(ContextBlockKind::Text, &features, None);

        let rendered = format!("{features:?}{input:?}{signals:?}");

        assert!(!rendered.contains("secret"), "no content may reach Debug");
        assert!(rendered.contains("line_count"));
        assert!(rendered.contains("content_bytes"));
        Ok(())
    }
}
