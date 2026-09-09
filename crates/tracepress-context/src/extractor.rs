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

use std::{collections::HashMap, fmt};

use serde::{Deserialize, Serialize};

use crate::span::{JsonValueKind, SpanNode, SpanNodeId};
use crate::visibility_analysis::{
    ContextVisibilityFacts, NoObservedResponseIds, ObservedResponseIdLookup, VisibilityObservation,
    derive_visibility,
};
use crate::{
    BlockContentMetadata, BlockFeatureInput, BlockFeatures, BlockLocator, BoundedMetadataText,
    ContextAnalysisLimitField, ContextAnalysisLimits, ContextAnalysisStatus, ContextBlockKind,
    ContextDigest, ContextOrigin, ContextRole, DetectionResult, EstimationRequest,
    OpportunitySignalInput, OpportunitySignalSet, RawSpanIndex, SemanticFingerprint,
    SemanticFingerprintInput, ShadowContentDetector, StructuralContentDetector,
    StructuralHeuristicEstimator, TokenEstimate, TokenEstimation, TokenEstimator,
    decode_json_string, derive_opportunity_signals, exact_fingerprint, extract_block_features,
    semantic_fingerprint,
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
    let index = RawSpanIndex::build(request, limits);
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

    fn add(&mut self, candidate: Candidate) {
        self.candidates.push(candidate);
    }
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

    let mut candidates = state.candidates;
    candidates.sort_by_key(|candidate| {
        index
            .node(candidate.source)
            .map_or((u64::MAX, u64::MAX), |node| {
                (node.span().start(), node.span().end())
            })
    });

    let mut blocks = Vec::new();
    let mut ordinal_by_source = HashMap::new();
    let mut extraction_reason = state.reason;
    for candidate in candidates {
        if blocks.len() >= limits.max_blocks.get() {
            let _ = extraction_reason.get_or_insert(ContextAnalysisReason::ResourceLimit {
                limit: ContextAnalysisLimitReason::Blocks,
            });
            break;
        }
        let Some(source) = index.node(candidate.source) else {
            continue;
        };
        if !source.is_complete() {
            continue;
        }
        if source.span().slice(request).is_none() {
            continue;
        }
        let Some(exact_fingerprint) = exact_fingerprint(request, source.span()) else {
            continue;
        };
        let parent_ordinal = candidate
            .parent_source
            .and_then(|parent| ordinal_by_source.get(&parent).copied());
        let ordinal = u32::try_from(blocks.len()).unwrap_or(u32::MAX);
        let metadata = metadata_for_candidate(request, limits, index, &candidate);
        let measurements =
            measure_candidate(request, limits, index, &candidate, model.as_deref(), source);
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
            token_estimate: measurements.token_estimate,
            detection_result: measurements.detection_result,
            features: measurements.features,
            opportunity_signals: measurements.opportunity_signals,
            tool_call_id: metadata.0,
            tool_name: metadata.1,
        };
        let _ = ordinal_by_source.insert(candidate.source, ordinal);
        blocks.push(draft);
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
    if extraction_reason.is_some() && status == ContextAnalysisStatus::Complete {
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

fn metadata_for_candidate(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    candidate: &Candidate,
) -> (Option<BoundedMetadataText>, Option<BoundedMetadataText>) {
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
    let call_id = call_id_node
        .and_then(|id| index.node(id))
        .and_then(|node| decode_json_string(request, node.span(), limits).ok())
        .map(|value| BoundedMetadataText::tool_name(&value));
    let name = name_node
        .and_then(|id| index.node(id))
        .and_then(|node| decode_json_string(request, node.span(), limits).ok())
        .map(|value| BoundedMetadataText::tool_name(&value));
    (call_id, name)
}

struct Measurements {
    semantic_fingerprint: Option<SemanticFingerprint>,
    token_estimate: Option<TokenEstimate>,
    detection_result: Option<DetectionResult>,
    features: Option<BlockFeatures>,
    opportunity_signals: OpportunitySignalSet,
}

fn measure_candidate(
    request: &[u8],
    limits: ContextAnalysisLimits,
    index: &RawSpanIndex,
    candidate: &Candidate,
    model: Option<&str>,
    source: &SpanNode,
) -> Measurements {
    let Some(payload) = index.node(candidate.payload) else {
        return Measurements {
            semantic_fingerprint: None,
            token_estimate: None,
            detection_result: None,
            features: None,
            opportunity_signals: OpportunitySignalSet::EMPTY,
        };
    };
    let inspectable = matches!(
        candidate.kind,
        ContextBlockKind::Instructions
            | ContextBlockKind::Text
            | ContextBlockKind::ToolCall
            | ContextBlockKind::ToolResult
            | ContextBlockKind::ToolDefinition
    );
    if !inspectable {
        return Measurements {
            semantic_fingerprint: None,
            token_estimate: None,
            detection_result: None,
            features: None,
            opportunity_signals: OpportunitySignalSet::EMPTY,
        };
    }
    let decoded = if payload.kind() == JsonValueKind::String {
        decode_json_string(request, payload.span(), limits).ok()
    } else {
        None
    };
    let content = if payload.kind() == JsonValueKind::String {
        decoded.as_deref().map(str::as_bytes)
    } else {
        payload.span().slice(request)
    };
    let Some(content) = content else {
        return Measurements {
            semantic_fingerprint: None,
            token_estimate: None,
            detection_result: None,
            features: None,
            opportunity_signals: OpportunitySignalSet::EMPTY,
        };
    };

    let detector = StructuralContentDetector::new(&limits);
    let detection_result = Some(detector.detect(
        content,
        BlockContentMetadata::new(candidate.kind).with_content_bytes(payload.raw_bytes()),
    ));
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
        semantic_fingerprint(SemanticFingerprintInput {
            request,
            block_kind: candidate.kind,
            role: candidate.role.unwrap_or(ContextRole::Unknown),
            value_span: payload.span(),
            value_kind: payload.kind(),
            duplicate_key_in_subtree: source.duplicate_key_detected(),
            fingerprint_version: crate::SEMANTIC_FINGERPRINT_VERSION,
            limits,
        })
    } else {
        None
    };
    Measurements {
        semantic_fingerprint,
        token_estimate,
        detection_result,
        features,
        opportunity_signals,
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
