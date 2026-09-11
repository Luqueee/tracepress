//! Bounded, span-preserving extraction of `OpenAI` Responses v1 context blocks.
//!
//! The extractor is deliberately an observer. It indexes and classifies values in the original
//! request bytes, then borrows safe content spans while running the crate's pure measurement
//! modules. No decoded prompt, tool argument, schema, URL, file content, or provider state is
//! retained in the returned analysis.
#![allow(
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "Responses extraction keeps independent visibility flags and passes the bounded request/index context explicitly"
)]

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt};
use tracepress_core::{
    ProcessingBudget, ProcessingBudgetAxis, ProcessingBudgetDecision, ProcessingCharge,
};

use crate::semantic_fingerprint_from_decoded;
use crate::span::{AnalysisClock, JsonValueKind, SpanNode, SpanNodeId};
use crate::visibility_analysis::{
    ContextVisibilityFacts, NoObservedResponseIds, ObservedResponseIdLookup, VisibilityObservation,
    derive_visibility,
};
use crate::{
    BlockContentMetadata, BlockFeatureInput, BlockFeatures, BlockLocator, BoundedMetadataText,
    ContextAnalysisLimitField, ContextAnalysisLimits, ContextAnalysisStatus, ContextBlockKind,
    ContextDigest, ContextOrigin, ContextRole, DetectionResult, EstimationRequest, MonotonicClock,
    OpportunitySignalInput, OpportunitySignalSet, RawSpan, RawSpanIndex, SemanticFingerprint,
    SemanticFingerprintInput, ShadowContentDetector, StructuralContentDetector,
    StructuralHeuristicEstimator, TokenEstimate, TokenEstimation, TokenEstimator,
    decode_json_string, derive_opportunity_signals, exact_fingerprint, extract_block_features,
};

/// Versioned reason an analysis stopped or was degraded.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextAnalysisLimitReason {
    /// Request bytes exceeded the admitted analysis window.
    AnalyzedBytes,
    /// The indexed value/block bound was reached.
    Blocks,
    /// JSON nesting exceeded the configured depth.
    JsonDepth,
    /// A string decode exceeded the inspection bound.
    StringBytesInspected,
    /// The abstract analysis work budget was exhausted.
    AnalysisWorkUnits,
    /// The analysis wall-clock budget was exhausted.
    AnalysisWallTimeMs,
    /// The bounded batch budget was reached.
    Batches,
}

impl From<ContextAnalysisLimitField> for ContextAnalysisLimitReason {
    fn from(field: ContextAnalysisLimitField) -> Self {
        match field {
            ContextAnalysisLimitField::AnalyzedBytes => Self::AnalyzedBytes,
            ContextAnalysisLimitField::Blocks => Self::Blocks,
            ContextAnalysisLimitField::JsonDepth => Self::JsonDepth,
            ContextAnalysisLimitField::StringBytesInspected => Self::StringBytesInspected,
            ContextAnalysisLimitField::AnalysisWorkUnits => Self::AnalysisWorkUnits,
            ContextAnalysisLimitField::AnalysisWallTimeMs => Self::AnalysisWallTimeMs,
            ContextAnalysisLimitField::Batches => Self::Batches,
        }
    }
}

/// Compact reason explaining a non-complete result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum ContextAnalysisReason {
    /// A configured bound stopped indexing or extraction.
    ResourceLimit {
        /// The bound that stopped the work.
        limit: ContextAnalysisLimitReason,
    },
    /// The request was not a valid JSON/UTF-8/NUL-safe input for this analysis.
    Malformed,
    /// Duplicate keys made at least one semantic path ambiguous.
    DuplicateKey,
    /// A known field had a shape this version cannot safely interpret.
    InvalidKnownShape,
    /// A context-bearing item type was not recognized by this version.
    UnknownContextItem,
}

/// Whether a bounded measurement path applies to one extracted block.
///
/// `Eligible` is structural applicability, not successful classification: a later work or string
/// budget may leave the corresponding observation absent. `Unavailable` is reserved for an
/// inspectable block whose content view was malformed or unreadable; references and opaque kinds
/// are `Ineligible`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MeasurementApplicability {
    /// The structural kind is not inspected by this measurement.
    Ineligible,
    /// A valid bounded content view exists, even if a later budget stops the measurement.
    Eligible,
    /// The structural kind is inspectable, but its content view was invalid or unreadable.
    Unavailable,
}

/// A context block before daemon identity allocation.
///
/// The locator and raw byte count describe the exact original JSON value. All other fields are
/// compact metadata; content is borrowed only while computing the optional measurements. The
/// `tool_call_id` and `tool_name` values are bounded metadata, not raw payloads.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ContextBlockDraft {
    /// Document order among emitted blocks.
    pub ordinal: u32,
    /// Ordinal of the enclosing emitted block, when one exists.
    pub parent_ordinal: Option<u32>,
    /// Canonical block kind.
    pub kind: ContextBlockKind,
    /// Role, kept orthogonal to kind and absent for non-conversational references.
    pub role: Option<ContextRole>,
    /// Structural provenance of the block.
    pub origin: ContextOrigin,
    /// Exact location in the original request.
    pub locator: BlockLocator,
    /// Bytes in the exact raw span.
    pub raw_bytes: u64,
    /// SHA-256 over the exact raw span bytes.
    pub exact_fingerprint: ContextDigest,
    /// Versioned semantic digest where equivalence is unambiguous.
    pub semantic_fingerprint: Option<SemanticFingerprint>,
    /// Whether semantic detection applies independently of whether it ran.
    pub detection_applicability: MeasurementApplicability,
    /// Whether token estimation applies independently of whether it ran.
    pub token_estimation_applicability: MeasurementApplicability,
    /// Bounded local token estimate, when applicable.
    pub token_estimate: Option<TokenEstimate>,
    /// Structural detector result, when a safe content span was inspected.
    pub detection_result: Option<DetectionResult>,
    /// Bounded aggregate features, when a safe content span was inspected.
    pub features: Option<BlockFeatures>,
    /// Opportunity observations derived from the bounded features.
    pub opportunity_signals: OpportunitySignalSet,
    /// Bounded tool-call correlation id, never the complete untrusted string.
    pub tool_call_id: Option<BoundedMetadataText>,
    /// Bounded tool name or MCP server label.
    pub tool_name: Option<BoundedMetadataText>,
}

impl fmt::Debug for ContextBlockDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextBlockDraft")
            .field("ordinal", &self.ordinal)
            .field("parent_ordinal", &self.parent_ordinal)
            .field("kind", &self.kind)
            .field("role", &self.role)
            .field("origin", &self.origin)
            .field("locator", &self.locator)
            .field("raw_bytes", &self.raw_bytes)
            .field("exact_fingerprint", &self.exact_fingerprint)
            .field("semantic_fingerprint", &self.semantic_fingerprint)
            .field("detection_applicability", &self.detection_applicability)
            .field(
                "token_estimation_applicability",
                &self.token_estimation_applicability,
            )
            .field("token_estimate", &self.token_estimate)
            .field("detection_result", &self.detection_result)
            .field("features", &self.features)
            .field("opportunity_signals", &self.opportunity_signals)
            .field("tool_call_id_present", &self.tool_call_id.is_some())
            .field("tool_name_present", &self.tool_name.is_some())
            .finish()
    }
}

/// Complete compact result of one `/v1/responses` request analysis.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ContextAnalysis {
    /// Terminal analysis status.
    pub status: ContextAnalysisStatus,
    /// Independent visibility signals; logical status is derived from these signals.
    pub visibility: crate::ContextVisibility,
    /// Whether any duplicate object key was observed.
    pub duplicate_key_detected: bool,
    /// SHA-256 over the exact accepted request bytes.
    pub request_content_hash: ContextDigest,
    /// SHA-256 over the bytes actually presented to the context analyzer.
    pub analysis_content_hash: ContextDigest,
    /// Bytes structurally inspected by the span indexer.
    pub analyzed_bytes: u64,
    /// Bytes forwarded but not structurally inspected.
    pub skipped_bytes: u64,
    /// Ordered block drafts, without daemon-owned ids.
    pub blocks: Vec<ContextBlockDraft>,
    /// Aggregate visibility and local-reference facts.
    pub visibility_facts: ContextVisibilityFacts,
    /// Why the result is not a fully complete analysis, when applicable.
    pub reason: Option<ContextAnalysisReason>,
}

impl fmt::Debug for ContextAnalysis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextAnalysis")
            .field("status", &self.status)
            .field("visibility", &self.visibility)
            .field("duplicate_key_detected", &self.duplicate_key_detected)
            .field("request_content_hash", &self.request_content_hash)
            .field("analysis_content_hash", &self.analysis_content_hash)
            .field("analyzed_bytes", &self.analyzed_bytes)
            .field("skipped_bytes", &self.skipped_bytes)
            .field("block_count", &self.blocks.len())
            .field("visibility_facts", &self.visibility_facts)
            .field("reason", &self.reason)
            .finish()
    }
}

impl ContextAnalysis {
    /// Returns the logical context claim derived from the independent visibility signals.
    #[must_use]
    pub const fn logical_context_status(&self) -> crate::LogicalContextStatus {
        self.visibility.logical_context_status()
    }

    /// Compatibility accessor using the domain snapshot's field name.
    #[must_use]
    pub const fn request_digest(&self) -> ContextDigest {
        self.request_content_hash
    }

    /// Returns the number of emitted drafts as a bounded `u32`.
    #[must_use]
    pub fn explicit_block_count(&self) -> u32 {
        u32::try_from(self.blocks.len()).unwrap_or(u32::MAX)
    }
}

/// Alias kept explicit for callers that distinguish an extractor result from its domain value.
pub type ContextAnalysisResult = ContextAnalysis;

/// Extracts a Responses v1 request without a local provider-response lookup.
#[must_use]
pub fn analyze(request: &[u8], limits: ContextAnalysisLimits) -> ContextAnalysisResult {
    analyze_responses_with_lookup::<NoObservedResponseIds>(request, limits, None)
}

/// Compatibility spelling for the Responses v1 analyzer.
#[must_use]
pub fn analyze_responses(request: &[u8], limits: ContextAnalysisLimits) -> ContextAnalysisResult {
    analyze(request, limits)
}

/// Extracts a Responses v1 request and uses a caller-supplied bounded response-id lookup.
#[must_use]
pub fn analyze_with_lookup<Lookup>(
    request: &[u8],
    limits: ContextAnalysisLimits,
    lookup: &Lookup,
) -> ContextAnalysisResult
where
    Lookup: ObservedResponseIdLookup + ?Sized,
{
    analyze_responses_with_lookup(request, limits, Some(lookup))
}

/// Compatibility spelling for [`analyze_with_lookup`].
#[must_use]
pub fn analyze_responses_with_lookup<Lookup>(
    request: &[u8],
    limits: ContextAnalysisLimits,
    lookup: Option<&Lookup>,
) -> ContextAnalysisResult
where
    Lookup: ObservedResponseIdLookup + ?Sized,
{
    let index = RawSpanIndex::build_for_context_analysis(request, limits);
    extract_indexed(request, limits, &index, lookup)
}

#[derive(Clone, Copy)]
struct Candidate {
    source: SpanNodeId,
    payload: SpanNodeId,
    parent_source: Option<SpanNodeId>,
    kind: ContextBlockKind,
    role: Option<ContextRole>,
    origin: ContextOrigin,
    metadata_owner: SpanNodeId,
    tool_call_id: Option<SpanNodeId>,
    tool_name: Option<SpanNodeId>,
}

#[derive(Default)]
struct ExtractionState {
    candidates: Vec<Candidate>,
    partial: bool,
    reason: Option<ContextAnalysisReason>,
    uses_previous_response: bool,
    previous_response_id: Option<SpanNodeId>,
    uses_conversation_state: bool,
    uses_item_references: bool,
    uses_prompt_reference: bool,
    uses_external_files: bool,
    uses_external_images: bool,
    contains_opaque_items: bool,
    facts: ContextVisibilityFacts,
}

impl ExtractionState {
    const fn mark_partial(&mut self, reason: ContextAnalysisReason) {
        self.partial = true;
        if self.reason.is_none() {
            self.reason = Some(reason);
        }
    }

    const fn mark_resource_limit(&mut self, limit: ContextAnalysisLimitReason) {
        self.partial = true;
        if !matches!(
            self.reason,
            Some(ContextAnalysisReason::ResourceLimit { .. })
        ) {
            self.reason = Some(ContextAnalysisReason::ResourceLimit { limit });
        }
    }

    fn add(&mut self, candidate: Candidate) {
        self.candidates.push(candidate);
    }
}
/// One charged measurement budget layered after the structural span scan.
///
/// Candidate spans may overlap, so each operation charges the bytes it is about to hash, decode,
/// or inspect before the operation starts. The cumulative byte total is converted to the same
/// coarse work units as the structural scanner, and every operation also samples wall time.
struct ExtractionBudget {
    budget: ProcessingBudget,
    clock: MonotonicClock,
    charged_ms: u64,
    charged_bytes: u64,
    exhausted: Option<ContextAnalysisLimitReason>,
}

impl ExtractionBudget {
    const BYTES_PER_WORK_UNIT: u64 = 4_096;

    fn new(limits: ContextAnalysisLimits) -> Self {
        Self {
            budget: limits.processing_budget(),
            clock: MonotonicClock::started_now(),
            charged_ms: 0,
            charged_bytes: 0,
            exhausted: None,
        }
    }

    fn charge_bytes(&mut self, bytes: u64) -> bool {
        if self.exhausted.is_some() {
            return false;
        }
        let next_bytes = self.charged_bytes.saturating_add(bytes);
        let previous_units = work_units(self.charged_bytes, Self::BYTES_PER_WORK_UNIT);
        let next_units = work_units(next_bytes, Self::BYTES_PER_WORK_UNIT);
        let additional_units = next_units.saturating_sub(previous_units);
        let observed_ms = self.clock.elapsed_ms();
        let elapsed_ms = observed_ms.saturating_sub(self.charged_ms);
        match self
            .budget
            .charge(ProcessingCharge::new(elapsed_ms, additional_units))
        {
            ProcessingBudgetDecision::Continue { .. } => {
                self.charged_ms = observed_ms;
                self.charged_bytes = next_bytes;
                true
            }
            ProcessingBudgetDecision::Exhausted { axis, .. } => {
                self.exhausted = Some(match axis {
                    ProcessingBudgetAxis::Time => ContextAnalysisLimitReason::AnalysisWallTimeMs,
                    ProcessingBudgetAxis::CpuWork => ContextAnalysisLimitReason::AnalysisWorkUnits,
                });
                false
            }
        }
    }

    const fn reason(&self) -> Option<ContextAnalysisLimitReason> {
        self.exhausted
    }

    fn is_exhausted(&mut self) -> bool {
        if self.exhausted.is_some() {
            return true;
        }
        !self.charge_bytes(0)
    }
}

fn work_units(value: u64, per_unit: u64) -> u64 {
    if value == 0 {
        return 0;
    }
    value
        .saturating_add(per_unit.saturating_sub(1))
        .checked_div(per_unit)
        .unwrap_or(u64::MAX)
}

fn extract_indexed<Lookup>(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    lookup: Option<&Lookup>,
) -> ContextAnalysisResult
where
    Lookup: ObservedResponseIdLookup + ?Sized,
{
    let request_content_hash = ContextDigest::from_bytes(request);
    let analysis_content_hash = request_content_hash;
    let mut state = ExtractionState::default();
    let root = index.root();
    if let Some(root) = root {
        if root.kind() == JsonValueKind::Object {
            collect_root(request, limits, index, root, &mut state);
        } else {
            state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
        }
    } else if index.status() != ContextAnalysisStatus::ResourceLimit {
        state.mark_partial(ContextAnalysisReason::Malformed);
    }

    let mut model = None;
    if let Some(root) = root.filter(|node| node.kind() == JsonValueKind::Object) {
        if let Some(model_node) = first_named(index, request, root.id(), "model") {
            model = decode_json_string(request, model_node.span(), limits).ok();
        }
    }

    let mut candidates = std::mem::take(&mut state.candidates);
    candidates.sort_by_key(|candidate| {
        index
            .node(candidate.source)
            .map_or((u64::MAX, u64::MAX), |node| {
                (node.span().start(), node.span().end())
            })
    });

    let mut blocks = Vec::new();
    let mut ordinal_by_source = HashMap::new();
    let mut exact_fingerprints = HashMap::<RawSpan, ContextDigest>::new();
    let mut extraction_budget = ExtractionBudget::new(limits);
    let mut extraction_reason = state.reason;
    for candidate in candidates {
        if blocks.len() >= limits.max_blocks.get() {
            state.mark_resource_limit(ContextAnalysisLimitReason::Blocks);
            extraction_reason = state.reason;
            break;
        }
        if extraction_budget.is_exhausted() {
            if let Some(limit) = extraction_budget.reason() {
                state.mark_resource_limit(limit);
            }
            extraction_reason = state.reason;
            break;
        }
        let Some(source) = index.node(candidate.source) else {
            continue;
        };
        if !source.is_complete() {
            continue;
        }
        let source_span = source.span();
        if source_span.slice(request).is_none() {
            continue;
        }
        let exact_fingerprint = if let Some(cached) = exact_fingerprints.get(&source_span) {
            *cached
        } else {
            if !extraction_budget.charge_bytes(source_span.len_bytes()) {
                if let Some(limit) = extraction_budget.reason() {
                    state.mark_resource_limit(limit);
                }
                extraction_reason = state.reason;
                break;
            }
            let Some(exact_fingerprint) = exact_fingerprint(request, source_span) else {
                continue;
            };
            let _ = exact_fingerprints.insert(source_span, exact_fingerprint);
            exact_fingerprint
        };
        let parent_ordinal = candidate
            .parent_source
            .and_then(|parent| ordinal_by_source.get(&parent).copied());
        let ordinal = u32::try_from(blocks.len()).unwrap_or(u32::MAX);
        let metadata =
            metadata_for_candidate(request, limits, index, &candidate, &mut extraction_budget);
        if let Some(limit) = metadata.limit {
            state.mark_resource_limit(limit);
        }
        let measurements = if extraction_budget.is_exhausted() {
            Measurements::for_applicability(candidate_measurement_applicability(
                request, limits, index, &candidate,
            ))
        } else {
            let outcome = measure_candidate(
                request,
                limits,
                index,
                &candidate,
                model.as_deref(),
                source,
                &mut extraction_budget,
            );
            if let Some(limit) = outcome.limit {
                state.mark_resource_limit(limit);
            }
            outcome.measurements
        };
        let draft = ContextBlockDraft {
            ordinal,
            parent_ordinal,
            kind: candidate.kind,
            role: candidate.role,
            origin: candidate.origin,
            locator: source.locator().clone(),
            raw_bytes: source.raw_bytes(),
            exact_fingerprint,
            semantic_fingerprint: measurements.semantic_fingerprint,
            detection_applicability: measurements.detection_applicability,
            token_estimation_applicability: measurements.token_estimation_applicability,
            token_estimate: measurements.token_estimate,
            detection_result: measurements.detection_result,
            features: measurements.features,
            opportunity_signals: measurements.opportunity_signals,
            tool_call_id: metadata.tool_call_id,
            tool_name: metadata.tool_name,
        };
        let _ = ordinal_by_source.insert(candidate.source, ordinal);
        blocks.push(draft);
        if extraction_budget.is_exhausted() {
            if let Some(limit) = extraction_budget.reason() {
                state.mark_resource_limit(limit);
            }
            extraction_reason = state.reason;
            break;
        }
        extraction_reason = state.reason;
    }

    let mut status = match index.status() {
        ContextAnalysisStatus::Malformed => ContextAnalysisStatus::Malformed,
        ContextAnalysisStatus::ResourceLimit => ContextAnalysisStatus::ResourceLimit,
        ContextAnalysisStatus::Partial => ContextAnalysisStatus::Partial,
        ContextAnalysisStatus::Complete
        | ContextAnalysisStatus::ObserverBackpressure
        | ContextAnalysisStatus::CorrelationDegraded
        | ContextAnalysisStatus::Unsupported
        | ContextAnalysisStatus::Cancelled => ContextAnalysisStatus::Complete,
    };
    if state.partial && status == ContextAnalysisStatus::Complete {
        status = ContextAnalysisStatus::Partial;
    }
    if extraction_reason
        .is_some_and(|reason| matches!(reason, ContextAnalysisReason::ResourceLimit { .. }))
        && !matches!(status, ContextAnalysisStatus::Malformed)
    {
        status = ContextAnalysisStatus::ResourceLimit;
    } else if extraction_reason.is_some() && status == ContextAnalysisStatus::Complete {
        status = ContextAnalysisStatus::Partial;
    }
    let explicit_request_complete = status == ContextAnalysisStatus::Complete
        && index.skipped_bytes() == 0
        && !index.duplicate_key_detected();
    let previous_response_id = state
        .previous_response_id
        .and_then(|id| index.node(id))
        .and_then(|node| decode_json_string(request, node.span(), limits).ok());
    let visibility = derive_visibility(
        VisibilityObservation {
            explicit_request_complete,
            uses_previous_response: state.uses_previous_response,
            previous_response_id: previous_response_id.as_deref(),
            uses_conversation_state: state.uses_conversation_state,
            uses_item_references: state.uses_item_references,
            uses_prompt_reference: state.uses_prompt_reference,
            uses_external_files: state.uses_external_files,
            uses_external_images: state.uses_external_images,
            contains_opaque_items: state.contains_opaque_items,
            facts: state.facts,
        },
        lookup,
    );

    let reason = if matches!(status, ContextAnalysisStatus::Malformed) {
        Some(ContextAnalysisReason::Malformed)
    } else if let Some(limit) = index.limit_reached() {
        Some(ContextAnalysisReason::ResourceLimit {
            limit: limit.into(),
        })
    } else if index.duplicate_key_detected() && extraction_reason.is_none() {
        Some(ContextAnalysisReason::DuplicateKey)
    } else {
        extraction_reason
    };

    ContextAnalysis {
        status,
        visibility: visibility.visibility,
        duplicate_key_detected: index.duplicate_key_detected(),
        request_content_hash,
        analysis_content_hash,
        analyzed_bytes: index.analyzed_bytes(),
        skipped_bytes: index.skipped_bytes(),
        blocks,
        visibility_facts: visibility.facts,
        reason,
    }
}

fn collect_root(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    root: &SpanNode,
    state: &mut ExtractionState,
) {
    for child in index.children(root.id()) {
        if child.name_equals(request, "instructions") {
            if child.kind() == JsonValueKind::String {
                state.add(simple_candidate(
                    child,
                    ContextBlockKind::Instructions,
                    Some(ContextRole::System),
                    ContextOrigin::HumanAuthored,
                    root.id(),
                ));
            } else {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
        } else if child.name_equals(request, "input") {
            collect_input(request, limits, index, child, state);
        } else if child.name_equals(request, "tools") {
            if child.kind() == JsonValueKind::Array {
                for tool in index.children(child.id()) {
                    collect_tool_definition(request, index, tool, state);
                }
            } else {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
        } else if child.name_equals(request, "previous_response_id") {
            state.uses_previous_response = true;
            state.facts.add_provider_state_reference();
            let _ = state.previous_response_id.get_or_insert_with(|| child.id());
            if child.kind() == JsonValueKind::String {
                state.add(simple_candidate(
                    child,
                    ContextBlockKind::ProviderStateReference,
                    None,
                    ContextOrigin::ProviderManaged,
                    root.id(),
                ));
            } else {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
        } else if child.name_equals(request, "conversation") {
            state.uses_conversation_state = true;
            state.facts.add_conversation_reference();
            if child.kind() == JsonValueKind::String
                || child.kind() == JsonValueKind::Object
                || child.kind() == JsonValueKind::Array
            {
                state.add(simple_candidate(
                    child,
                    ContextBlockKind::ProviderStateReference,
                    None,
                    ContextOrigin::ProviderManaged,
                    root.id(),
                ));
            } else {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
        } else if child.name_equals(request, "prompt") {
            state.uses_prompt_reference = true;
            state.facts.add_prompt_reference();
            if !matches!(child.kind(), JsonValueKind::String | JsonValueKind::Object) {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
            state.add(simple_candidate(
                child,
                ContextBlockKind::PromptReference,
                None,
                ContextOrigin::ProviderManaged,
                root.id(),
            ));
        } else if child.name_equals(request, "item_reference")
            || child.name_equals(request, "item_ref")
            || child.name_equals(request, "item_references")
        {
            collect_reference_value(
                index,
                child,
                ContextBlockKind::ItemReference,
                root.id(),
                state,
            );
        } else if child.name_equals(request, "file_id") || child.name_equals(request, "file_url") {
            state.uses_external_files = true;
            state.facts.add_external_file_reference();
            if child.kind() != JsonValueKind::String {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
            state.add(simple_candidate(
                child,
                ContextBlockKind::FileReference,
                None,
                ContextOrigin::ExternalReference,
                root.id(),
            ));
        } else if child.name_equals(request, "image_url") || child.name_equals(request, "image_id")
        {
            state.uses_external_images = true;
            state.facts.add_external_image_reference();
            if child.kind() != JsonValueKind::String {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
            state.add(simple_candidate(
                child,
                ContextBlockKind::ImageReference,
                None,
                ContextOrigin::ExternalReference,
                root.id(),
            ));
        }
    }
}

fn collect_input(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    input: &SpanNode,
    state: &mut ExtractionState,
) {
    match input.kind() {
        JsonValueKind::String => state.add(simple_candidate(
            input,
            ContextBlockKind::Text,
            Some(ContextRole::User),
            ContextOrigin::HumanAuthored,
            input.parent().unwrap_or_else(|| input.id()),
        )),
        JsonValueKind::Array => {
            for item in index.children(input.id()) {
                collect_item(request, limits, index, item, state);
            }
        }
        JsonValueKind::Object
        | JsonValueKind::Number
        | JsonValueKind::Boolean
        | JsonValueKind::Null => {
            state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
        }
    }
}

fn collect_item(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    item: &SpanNode,
    state: &mut ExtractionState,
) {
    if item.kind() != JsonValueKind::Object {
        state.mark_partial(ContextAnalysisReason::UnknownContextItem);
        state.add(simple_candidate(
            item,
            ContextBlockKind::Unknown,
            None,
            ContextOrigin::Unknown,
            item.parent().unwrap_or_else(|| item.id()),
        ));
        return;
    }
    let item_type = first_named(index, request, item.id(), "type")
        .and_then(|node| decode_json_string(request, node.span(), limits).ok());
    let role = first_named(index, request, item.id(), "role")
        .and_then(|node| decode_json_string(request, node.span(), limits).ok())
        .map(|value| role_from_str(&value));
    let origin = origin_for_role(role);
    match item_type.as_deref() {
        Some("message") => {
            collect_message(request, limits, index, item, role, origin, state);
        }
        None if has_role_or_content(index, request, item.id()) => {
            collect_message(request, limits, index, item, role, origin, state);
        }
        Some("assistant_history") => {
            collect_message_kind(
                request,
                limits,
                index,
                item,
                ContextBlockKind::AssistantHistory,
                role.or(Some(ContextRole::Assistant)),
                ContextOrigin::AgentGenerated,
                state,
            );
        }
        Some(
            "function_call"
            | "computer_call"
            | "web_search_call"
            | "web_search_preview_call"
            | "code_interpreter_call",
        ) => {
            collect_tool_call(request, index, item, state);
        }
        Some(
            "function_call_output"
            | "computer_call_output"
            | "web_search_call_output"
            | "code_interpreter_call_output",
        ) => {
            collect_tool_result(request, index, item, state);
        }
        Some("item_reference" | "item_ref") => {
            state.uses_item_references = true;
            state.facts.add_item_reference();
            state.add(simple_candidate(
                item,
                ContextBlockKind::ItemReference,
                role,
                ContextOrigin::ProviderManaged,
                item.parent().unwrap_or_else(|| item.id()),
            ));
        }
        Some("prompt" | "prompt_reference") => {
            state.uses_prompt_reference = true;
            state.facts.add_prompt_reference();
            state.add(simple_candidate(
                item,
                ContextBlockKind::PromptReference,
                role,
                ContextOrigin::ProviderManaged,
                item.parent().unwrap_or_else(|| item.id()),
            ));
        }
        Some("reasoning" | "opaque_reasoning") => {
            state.contains_opaque_items = true;
            state.facts.add_opaque_item();
            state.add(simple_candidate(
                item,
                ContextBlockKind::OpaqueReasoning,
                role.or(Some(ContextRole::Assistant)),
                ContextOrigin::ProviderManaged,
                item.parent().unwrap_or_else(|| item.id()),
            ));
        }
        Some("input_text" | "output_text") => {
            collect_text_part(
                request,
                index,
                item,
                role.or(Some(ContextRole::User)),
                origin,
                None,
                state,
            );
        }
        Some(_) | None => {
            state.mark_partial(ContextAnalysisReason::UnknownContextItem);
            state.add(simple_candidate(
                item,
                ContextBlockKind::Unknown,
                role,
                ContextOrigin::Unknown,
                item.parent().unwrap_or_else(|| item.id()),
            ));
        }
    }
}

fn collect_message(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    item: &SpanNode,
    role: Option<ContextRole>,
    origin: ContextOrigin,
    state: &mut ExtractionState,
) {
    collect_message_kind(
        request,
        limits,
        index,
        item,
        ContextBlockKind::Message,
        role,
        origin,
        state,
    );
}

fn collect_message_kind(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    item: &SpanNode,
    kind: ContextBlockKind,
    role: Option<ContextRole>,
    origin: ContextOrigin,
    state: &mut ExtractionState,
) {
    state.add(simple_candidate(
        item,
        kind,
        role,
        origin,
        item.parent().unwrap_or_else(|| item.id()),
    ));
    for content in named_children(index, request, item.id(), "content") {
        match content.kind() {
            JsonValueKind::String => state.add(Candidate {
                source: content.id(),
                payload: content.id(),
                parent_source: Some(item.id()),
                kind: ContextBlockKind::Text,
                role,
                origin,
                metadata_owner: item.id(),
                tool_call_id: None,
                tool_name: None,
            }),
            JsonValueKind::Array => {
                for part in index.children(content.id()) {
                    collect_content_part(
                        request,
                        limits,
                        index,
                        part,
                        item.id(),
                        role,
                        origin,
                        state,
                    );
                }
            }
            JsonValueKind::Object => {
                collect_content_part(
                    request,
                    limits,
                    index,
                    content,
                    item.id(),
                    role,
                    origin,
                    state,
                );
            }
            _ => state.mark_partial(ContextAnalysisReason::InvalidKnownShape),
        }
    }
}

fn collect_content_part(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    part: &SpanNode,
    parent: SpanNodeId,
    role: Option<ContextRole>,
    origin: ContextOrigin,
    state: &mut ExtractionState,
) {
    if part.kind() != JsonValueKind::Object {
        state.mark_partial(ContextAnalysisReason::UnknownContextItem);
        state.add(Candidate {
            source: part.id(),
            payload: part.id(),
            parent_source: Some(parent),
            kind: ContextBlockKind::Unknown,
            role,
            origin: ContextOrigin::Unknown,
            metadata_owner: part.id(),
            tool_call_id: None,
            tool_name: None,
        });
        return;
    }
    let part_type = first_named(index, request, part.id(), "type")
        .and_then(|node| decode_json_string(request, node.span(), limits).ok());
    match part_type.as_deref() {
        Some("input_text" | "output_text" | "text") | None
            if first_named(index, request, part.id(), "text").is_some() =>
        {
            collect_text_part(request, index, part, role, origin, Some(parent), state);
        }
        Some("input_image" | "output_image" | "image") => {
            state.uses_external_images = true;
            state.facts.add_external_image_reference();
            state.add(Candidate {
                source: part.id(),
                payload: part.id(),
                parent_source: Some(parent),
                kind: ContextBlockKind::ImageReference,
                role,
                origin: ContextOrigin::ExternalReference,
                metadata_owner: part.id(),
                tool_call_id: None,
                tool_name: None,
            });
        }
        Some("input_file" | "output_file" | "file") => {
            state.uses_external_files = true;
            state.facts.add_external_file_reference();
            state.add(Candidate {
                source: part.id(),
                payload: part.id(),
                parent_source: Some(parent),
                kind: ContextBlockKind::FileReference,
                role,
                origin: ContextOrigin::ExternalReference,
                metadata_owner: part.id(),
                tool_call_id: None,
                tool_name: None,
            });
        }
        Some("item_reference" | "item_ref") => {
            state.uses_item_references = true;
            state.facts.add_item_reference();
            state.add(Candidate {
                source: part.id(),
                payload: part.id(),
                parent_source: Some(parent),
                kind: ContextBlockKind::ItemReference,
                role,
                origin: ContextOrigin::ProviderManaged,
                metadata_owner: part.id(),
                tool_call_id: None,
                tool_name: None,
            });
        }
        Some("prompt" | "prompt_reference") => {
            state.uses_prompt_reference = true;
            state.facts.add_prompt_reference();
            state.add(Candidate {
                source: part.id(),
                payload: part.id(),
                parent_source: Some(parent),
                kind: ContextBlockKind::PromptReference,
                role,
                origin: ContextOrigin::ProviderManaged,
                metadata_owner: part.id(),
                tool_call_id: None,
                tool_name: None,
            });
        }
        Some("reasoning" | "opaque_reasoning") => {
            state.contains_opaque_items = true;
            state.facts.add_opaque_item();
            state.add(Candidate {
                source: part.id(),
                payload: part.id(),
                parent_source: Some(parent),
                kind: ContextBlockKind::OpaqueReasoning,
                role,
                origin: ContextOrigin::ProviderManaged,
                metadata_owner: part.id(),
                tool_call_id: None,
                tool_name: None,
            });
        }
        Some(_) | None => {
            state.mark_partial(ContextAnalysisReason::UnknownContextItem);
            state.add(Candidate {
                source: part.id(),
                payload: part.id(),
                parent_source: Some(parent),
                kind: ContextBlockKind::Unknown,
                role,
                origin: ContextOrigin::Unknown,
                metadata_owner: part.id(),
                tool_call_id: None,
                tool_name: None,
            });
        }
    }
}

fn collect_text_part(
    request: &[u8],
    index: &RawSpanIndex,
    part: &SpanNode,
    role: Option<ContextRole>,
    origin: ContextOrigin,
    parent: Option<SpanNodeId>,
    state: &mut ExtractionState,
) {
    if let Some(text) = index
        .children(part.id())
        .find(|node| node.name_span().is_some() && node.name_equals(request, "text"))
    {
        state.add(Candidate {
            source: text.id(),
            payload: text.id(),
            parent_source: parent,
            kind: ContextBlockKind::Text,
            role,
            origin,
            metadata_owner: part.id(),
            tool_call_id: None,
            tool_name: None,
        });
    } else {
        // `collect_text_part` is called only after a text member was established by the caller;
        // an absent/invalid text value is nevertheless a known-shape degradation.
        state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
        state.add(Candidate {
            source: part.id(),
            payload: part.id(),
            parent_source: parent,
            kind: ContextBlockKind::Unknown,
            role,
            origin: ContextOrigin::Unknown,
            metadata_owner: part.id(),
            tool_call_id: None,
            tool_name: None,
        });
    }
}

fn collect_tool_call(
    request: &[u8],
    index: &RawSpanIndex,
    item: &SpanNode,
    state: &mut ExtractionState,
) {
    let id = first_named(index, request, item.id(), "call_id")
        .or_else(|| first_named(index, request, item.id(), "id"));
    let name = first_named(index, request, item.id(), "name");
    let payload = first_named(index, request, item.id(), "arguments")
        .or_else(|| first_named(index, request, item.id(), "input"))
        .unwrap_or(item);
    state.add(Candidate {
        source: item.id(),
        payload: payload.id(),
        parent_source: None,
        kind: ContextBlockKind::ToolCall,
        role: Some(ContextRole::Tool),
        origin: ContextOrigin::AgentGenerated,
        metadata_owner: item.id(),
        tool_call_id: id.map(SpanNode::id),
        tool_name: name.map(SpanNode::id),
    });
}

fn collect_tool_result(
    request: &[u8],
    index: &RawSpanIndex,
    item: &SpanNode,
    state: &mut ExtractionState,
) {
    let id = first_named(index, request, item.id(), "call_id")
        .or_else(|| first_named(index, request, item.id(), "id"));
    let name = first_named(index, request, item.id(), "name");
    let payload = first_named(index, request, item.id(), "output")
        .or_else(|| first_named(index, request, item.id(), "content"))
        .unwrap_or(item);
    state.add(Candidate {
        source: item.id(),
        payload: payload.id(),
        parent_source: None,
        kind: ContextBlockKind::ToolResult,
        role: Some(ContextRole::Tool),
        origin: ContextOrigin::ToolGenerated,
        metadata_owner: item.id(),
        tool_call_id: id.map(SpanNode::id),
        tool_name: name.map(SpanNode::id),
    });
}

fn collect_tool_definition(
    request: &[u8],
    index: &RawSpanIndex,
    tool: &SpanNode,
    state: &mut ExtractionState,
) {
    if tool.kind() != JsonValueKind::Object {
        state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
        state.add(simple_candidate(
            tool,
            ContextBlockKind::Unknown,
            None,
            ContextOrigin::Unknown,
            tool.parent().unwrap_or_else(|| tool.id()),
        ));
        return;
    }
    let name = first_named(index, request, tool.id(), "name")
        .or_else(|| {
            first_named(index, request, tool.id(), "function")
                .and_then(|function| first_named(index, request, function.id(), "name"))
        })
        .or_else(|| first_named(index, request, tool.id(), "server_label"));
    state.add(Candidate {
        source: tool.id(),
        payload: tool.id(),
        parent_source: None,
        kind: ContextBlockKind::ToolDefinition,
        role: None,
        origin: ContextOrigin::ToolSchema,
        metadata_owner: tool.id(),
        tool_call_id: None,
        tool_name: name.map(SpanNode::id),
    });
}

fn collect_reference_value(
    index: &RawSpanIndex,
    value: &SpanNode,
    kind: ContextBlockKind,
    parent: SpanNodeId,
    state: &mut ExtractionState,
) {
    if value.kind() == JsonValueKind::Array {
        for child in index.children(value.id()) {
            if !matches!(child.kind(), JsonValueKind::String | JsonValueKind::Object) {
                state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
            }
            state.uses_item_references = true;
            state.facts.add_item_reference();
            state.add(simple_candidate(
                child,
                kind,
                None,
                ContextOrigin::ProviderManaged,
                parent,
            ));
        }
    } else {
        if !matches!(value.kind(), JsonValueKind::String | JsonValueKind::Object) {
            state.mark_partial(ContextAnalysisReason::InvalidKnownShape);
        }
        state.uses_item_references = true;
        state.facts.add_item_reference();
        state.add(simple_candidate(
            value,
            kind,
            None,
            ContextOrigin::ProviderManaged,
            parent,
        ));
    }
}

const fn simple_candidate(
    node: &SpanNode,
    kind: ContextBlockKind,
    role: Option<ContextRole>,
    origin: ContextOrigin,
    _parent: SpanNodeId,
) -> Candidate {
    Candidate {
        source: node.id(),
        payload: node.id(),
        parent_source: None,
        kind,
        role,
        origin,
        metadata_owner: node.id(),
        tool_call_id: None,
        tool_name: None,
    }
    // Parent is intentionally not inferred from the structural root: only explicit nested blocks
    // (message -> content) establish a parent ordinal.
}

struct MetadataOutcome {
    tool_call_id: Option<BoundedMetadataText>,
    tool_name: Option<BoundedMetadataText>,
    limit: Option<ContextAnalysisLimitReason>,
}

fn metadata_for_candidate(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    candidate: &Candidate,
    budget: &mut ExtractionBudget,
) -> MetadataOutcome {
    let owner = index.node(candidate.metadata_owner);
    let call_id_node = candidate.tool_call_id.or_else(|| {
        owner
            .and_then(|owner| first_named(index, request, owner.id(), "call_id"))
            .map(SpanNode::id)
    });
    let name_node = candidate.tool_name.or_else(|| {
        owner
            .and_then(|owner| first_named(index, request, owner.id(), "name"))
            .map(SpanNode::id)
    });
    let (tool_call_id, call_id_limit) =
        metadata_value(request, limits, index, call_id_node, budget);
    let (tool_name, name_limit) = metadata_value(request, limits, index, name_node, budget);
    MetadataOutcome {
        tool_call_id,
        tool_name,
        limit: call_id_limit.or(name_limit),
    }
}

fn metadata_value(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    node_id: Option<SpanNodeId>,
    budget: &mut ExtractionBudget,
) -> (
    Option<BoundedMetadataText>,
    Option<ContextAnalysisLimitReason>,
) {
    let Some(node) = node_id.and_then(|id| index.node(id)) else {
        return (None, None);
    };
    let admitted_bytes = node
        .raw_bytes()
        .min(u64::try_from(limits.max_string_bytes_inspected.get()).unwrap_or(u64::MAX));
    if !budget.charge_bytes(admitted_bytes) {
        return (None, budget.reason());
    }
    match decode_json_string(request, node.span(), limits) {
        Ok(value) => (Some(BoundedMetadataText::tool_name(&value)), None),
        Err(crate::JsonStringDecodeError::AboveInspectionBound { .. }) => {
            (None, Some(ContextAnalysisLimitReason::StringBytesInspected))
        }
        Err(_) => (None, None),
    }
}

struct Measurements {
    semantic_fingerprint: Option<SemanticFingerprint>,
    token_estimate: Option<TokenEstimate>,
    detection_result: Option<DetectionResult>,
    features: Option<BlockFeatures>,
    opportunity_signals: OpportunitySignalSet,
    detection_applicability: MeasurementApplicability,
    token_estimation_applicability: MeasurementApplicability,
}

impl Measurements {
    const fn ineligible() -> Self {
        Self {
            semantic_fingerprint: None,
            token_estimate: None,
            detection_result: None,
            features: None,
            opportunity_signals: OpportunitySignalSet::EMPTY,
            detection_applicability: MeasurementApplicability::Ineligible,
            token_estimation_applicability: MeasurementApplicability::Ineligible,
        }
    }

    const fn unavailable() -> Self {
        Self {
            semantic_fingerprint: None,
            token_estimate: None,
            detection_result: None,
            features: None,
            opportunity_signals: OpportunitySignalSet::EMPTY,
            detection_applicability: MeasurementApplicability::Unavailable,
            token_estimation_applicability: MeasurementApplicability::Unavailable,
        }
    }

    const fn eligible_unobserved() -> Self {
        Self {
            semantic_fingerprint: None,
            token_estimate: None,
            detection_result: None,
            features: None,
            opportunity_signals: OpportunitySignalSet::EMPTY,
            detection_applicability: MeasurementApplicability::Eligible,
            token_estimation_applicability: MeasurementApplicability::Eligible,
        }
    }

    const fn for_applicability(applicability: MeasurementApplicability) -> Self {
        match applicability {
            MeasurementApplicability::Ineligible => Self::ineligible(),
            MeasurementApplicability::Eligible => Self::eligible_unobserved(),
            MeasurementApplicability::Unavailable => Self::unavailable(),
        }
    }
}
fn candidate_measurement_applicability(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    candidate: &Candidate,
) -> MeasurementApplicability {
    let inspectable = matches!(
        candidate.kind,
        ContextBlockKind::Instructions
            | ContextBlockKind::Text
            | ContextBlockKind::ToolCall
            | ContextBlockKind::ToolResult
            | ContextBlockKind::ToolDefinition
    );
    if !inspectable {
        return MeasurementApplicability::Ineligible;
    }
    let Some(payload) = index.node(candidate.payload) else {
        return MeasurementApplicability::Unavailable;
    };
    if payload.kind() == JsonValueKind::String {
        if payload.raw_bytes()
            > u64::try_from(limits.max_string_bytes_inspected.get()).unwrap_or(u64::MAX)
        {
            return MeasurementApplicability::Eligible;
        }
        match decode_json_string(request, payload.span(), limits) {
            Ok(_) | Err(crate::JsonStringDecodeError::AboveInspectionBound { .. }) => {
                MeasurementApplicability::Eligible
            }
            Err(_) => MeasurementApplicability::Unavailable,
        }
    } else if payload.span().slice(request).is_some() {
        MeasurementApplicability::Eligible
    } else {
        MeasurementApplicability::Unavailable
    }
}

struct MeasurementOutcome {
    measurements: Measurements,
    limit: Option<ContextAnalysisLimitReason>,
}
fn measure_candidate(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    candidate: &Candidate,
    model: Option<&str>,
    source: &SpanNode,
    budget: &mut ExtractionBudget,
) -> MeasurementOutcome {
    let unavailable = || MeasurementOutcome {
        measurements: Measurements::unavailable(),
        limit: None,
    };
    let limited = |limit| MeasurementOutcome {
        measurements: Measurements::eligible_unobserved(),
        limit: Some(limit),
    };
    let Some(payload) = index.node(candidate.payload) else {
        return unavailable();
    };
    let content_applicability =
        candidate_measurement_applicability(request, limits, index, candidate);
    if content_applicability != MeasurementApplicability::Eligible {
        return MeasurementOutcome {
            measurements: Measurements::for_applicability(content_applicability),
            limit: None,
        };
    }
    let decoded = if payload.kind() == JsonValueKind::String {
        let admitted_bytes = payload
            .raw_bytes()
            .min(u64::try_from(limits.max_string_bytes_inspected.get()).unwrap_or(u64::MAX));
        if !budget.charge_bytes(admitted_bytes) {
            return limited(
                budget
                    .reason()
                    .unwrap_or(ContextAnalysisLimitReason::AnalysisWorkUnits),
            );
        }
        match decode_json_string(request, payload.span(), limits) {
            Ok(value) => Some(value),
            Err(crate::JsonStringDecodeError::AboveInspectionBound { .. }) => {
                return limited(ContextAnalysisLimitReason::StringBytesInspected);
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let content = if payload.kind() == JsonValueKind::String {
        decoded.as_deref().map(str::as_bytes)
    } else {
        payload.span().slice(request)
    };
    let Some(content) = content else {
        return unavailable();
    };

    let detector_bytes = u64::try_from(content.len().min(limits.max_string_bytes_inspected.get()))
        .unwrap_or(u64::MAX);
    if !budget.charge_bytes(detector_bytes) {
        return limited(
            budget
                .reason()
                .unwrap_or(ContextAnalysisLimitReason::AnalysisWorkUnits),
        );
    }
    let detector = StructuralContentDetector::new(&limits);
    let detection_result = Some(detector.detect(
        content,
        BlockContentMetadata::new(candidate.kind).with_content_bytes(payload.raw_bytes()),
    ));

    let estimator_bytes = u64::try_from(content.len())
        .unwrap_or(u64::MAX)
        .min(u64::try_from(limits.max_analyzed_bytes.get()).unwrap_or(u64::MAX));
    if !budget.charge_bytes(estimator_bytes) {
        return MeasurementOutcome {
            measurements: Measurements {
                semantic_fingerprint: None,
                token_estimate: None,
                detection_result,
                features: None,
                opportunity_signals: OpportunitySignalSet::EMPTY,
                detection_applicability: MeasurementApplicability::Eligible,
                token_estimation_applicability: MeasurementApplicability::Eligible,
            },
            limit: Some(
                budget
                    .reason()
                    .unwrap_or(ContextAnalysisLimitReason::AnalysisWorkUnits),
            ),
        };
    }
    let estimator = StructuralHeuristicEstimator::new();
    let token_estimate = match estimator.estimate(&EstimationRequest {
        model,
        content,
        limits,
    }) {
        TokenEstimation::Complete(estimate) | TokenEstimation::Bounded { estimate, .. } => {
            Some(estimate)
        }
        TokenEstimation::Unavailable(_) => None,
    };
    if !budget.charge_bytes(detector_bytes) {
        return MeasurementOutcome {
            measurements: Measurements {
                semantic_fingerprint: None,
                token_estimate,
                detection_result,
                features: None,
                opportunity_signals: OpportunitySignalSet::EMPTY,
                detection_applicability: MeasurementApplicability::Eligible,
                token_estimation_applicability: MeasurementApplicability::Eligible,
            },
            limit: Some(
                budget
                    .reason()
                    .unwrap_or(ContextAnalysisLimitReason::AnalysisWorkUnits),
            ),
        };
    }
    let features = Some(extract_block_features(&BlockFeatureInput {
        raw_bytes: source.raw_bytes(),
        content,
        content_truncated: false,
        detection: detection_result,
        limits,
    }));
    let opportunity_signals = features.map_or(OpportunitySignalSet::EMPTY, |features| {
        derive_opportunity_signals(&OpportunitySignalInput {
            kind: candidate.kind,
            features: &features,
            detection: detection_result,
            estimated_tokens: token_estimate.as_ref().map(|estimate| estimate.tokens),
            repeated_in_session: false,
        })
    });

    let semantic_fingerprint = if matches!(
        candidate.kind,
        ContextBlockKind::Text | ContextBlockKind::ToolResult
    ) && payload.kind() == JsonValueKind::String
    {
        let Some(decoded) = decoded.as_deref() else {
            return MeasurementOutcome {
                measurements: Measurements {
                    semantic_fingerprint: None,
                    token_estimate,
                    detection_result,
                    features,
                    opportunity_signals,
                    detection_applicability: MeasurementApplicability::Eligible,
                    token_estimation_applicability: MeasurementApplicability::Eligible,
                },
                limit: None,
            };
        };
        if !budget.charge_bytes(u64::try_from(decoded.len()).unwrap_or(u64::MAX)) {
            return MeasurementOutcome {
                measurements: Measurements {
                    semantic_fingerprint: None,
                    token_estimate,
                    detection_result,
                    features,
                    opportunity_signals,
                    detection_applicability: MeasurementApplicability::Eligible,
                    token_estimation_applicability: MeasurementApplicability::Eligible,
                },
                limit: Some(
                    budget
                        .reason()
                        .unwrap_or(ContextAnalysisLimitReason::AnalysisWorkUnits),
                ),
            };
        }
        semantic_fingerprint_from_decoded(
            SemanticFingerprintInput {
                request,
                block_kind: candidate.kind,
                role: candidate.role.unwrap_or(ContextRole::Unknown),
                value_span: payload.span(),
                value_kind: payload.kind(),
                duplicate_key_in_subtree: source.duplicate_key_detected(),
                fingerprint_version: crate::SEMANTIC_FINGERPRINT_VERSION,
                limits,
            },
            decoded,
        )
    } else {
        None
    };
    MeasurementOutcome {
        measurements: Measurements {
            semantic_fingerprint,
            token_estimate,
            detection_result,
            features,
            opportunity_signals,
            detection_applicability: MeasurementApplicability::Eligible,
            token_estimation_applicability: MeasurementApplicability::Eligible,
        },
        limit: None,
    }
}

fn first_named<'index>(
    index: &'index RawSpanIndex,
    request: &[u8],
    object: SpanNodeId,
    name: &str,
) -> Option<&'index SpanNode> {
    index
        .children(object)
        .find(|node| node.name_equals(request, name))
}

fn named_children<'index>(
    index: &'index RawSpanIndex,
    request: &[u8],
    object: SpanNodeId,
    name: &str,
) -> Vec<&'index SpanNode> {
    index
        .children(object)
        .filter(|node| node.name_equals(request, name))
        .collect()
}

fn has_role_or_content(index: &RawSpanIndex, request: &[u8], object: SpanNodeId) -> bool {
    first_named(index, request, object, "role").is_some()
        || first_named(index, request, object, "content").is_some()
}

fn role_from_str(value: &str) -> ContextRole {
    match value {
        "system" => ContextRole::System,
        "developer" => ContextRole::Developer,
        "user" => ContextRole::User,
        "assistant" => ContextRole::Assistant,
        "tool" => ContextRole::Tool,
        _ => ContextRole::Unknown,
    }
}

const fn origin_for_role(role: Option<ContextRole>) -> ContextOrigin {
    match role {
        Some(ContextRole::System | ContextRole::Developer | ContextRole::User) => {
            ContextOrigin::HumanAuthored
        }
        Some(ContextRole::Assistant) => ContextOrigin::AgentGenerated,
        Some(ContextRole::Tool) => ContextOrigin::ToolGenerated,
        Some(ContextRole::Unknown) | None => ContextOrigin::Unknown,
    }
}
