//! Visibility and provider-reference analysis for Responses v1 context extraction.
//!
//! This module only records facts visible in the request envelope. It never asks a provider for
//! state, fetches an external reference, or reconstructs an earlier request. A caller may supply a
//! bounded set/lookup of response ids it has already observed; the lookup is used only for the
//! presence-only local linkage fact.
#![allow(
    clippy::struct_excessive_bools,
    reason = "visibility keeps one independently observable boolean per contract signal"
)]

use std::{
    borrow::Borrow,
    collections::{BTreeSet, HashSet},
    fmt,
    hash::{BuildHasher, Hash},
};

use serde::{Deserialize, Serialize};

use crate::{ContextDigest, ContextVisibility};

/// A presence-only provider state reference.
///
/// The referenced id is deliberately not retained. `reference_hash` is a local digest used only
/// to correlate equal observed ids, while `observed_response_id_match` says whether the optional
/// caller-supplied observation lookup contained that id. Neither field permits reconstructing
/// provider history.
#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ProviderStateReference {
    /// Whether a provider-state reference was present in the request.
    pub present: bool,
    /// Digest of the observed response id, when one was present.
    pub reference_hash: Option<ContextDigest>,
    /// Result of the caller-supplied local observation lookup, when supplied.
    pub observed_response_id_match: Option<bool>,
}

impl fmt::Debug for ProviderStateReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderStateReference")
            .field("present", &self.present)
            .field("reference_hash", &self.reference_hash)
            .field(
                "observed_response_id_match",
                &self.observed_response_id_match,
            )
            .finish()
    }
}

impl ProviderStateReference {
    /// Returns an absent provider-state reference.
    #[must_use]
    pub const fn absent() -> Self {
        Self {
            present: false,
            reference_hash: None,
            observed_response_id_match: None,
        }
    }

    /// Builds a presence-only record from a decoded response id.
    #[must_use]
    pub fn from_response_id(response_id: &str, observed_match: Option<bool>) -> Self {
        Self {
            present: true,
            reference_hash: Some(ContextDigest::from_bytes(response_id.as_bytes())),
            observed_response_id_match: observed_match,
        }
    }
}

/// Aggregate reference facts produced alongside [`ContextVisibility`].
///
/// Counts are bounded by the extractor's block limit and are saturated rather than allowed to
/// wrap. The booleans in [`ContextVisibility`] remain the canonical signal set; these counters
/// are explanatory aggregates and never carry an independent logical status.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ContextVisibilityFacts {
    /// Number of previous-response or other provider-state references observed.
    pub provider_state_reference_count: u32,
    /// Number of provider-held item references observed.
    pub item_reference_count: u32,
    /// Number of provider prompt references observed.
    pub prompt_reference_count: u32,
    /// Number of provider conversation-state references observed.
    pub conversation_reference_count: u32,
    /// Number of external file references observed.
    pub external_file_reference_count: u32,
    /// Number of external image references observed.
    pub external_image_reference_count: u32,
    /// Number of opaque or reasoning items whose semantics are provider-managed.
    pub opaque_item_count: u32,
    /// Whether the optional response-id lookup resolved at least one local reference.
    pub reference_resolved_locally: bool,
    /// Presence-only details for the previous-response provider-state reference.
    pub provider_state_reference: Option<ProviderStateReference>,
}

impl ContextVisibilityFacts {
    /// Records one bounded occurrence in a saturating counter.
    pub(crate) const fn add_provider_state_reference(&mut self) {
        self.provider_state_reference_count = self.provider_state_reference_count.saturating_add(1);
    }

    /// Records one bounded item reference.
    pub(crate) const fn add_item_reference(&mut self) {
        self.item_reference_count = self.item_reference_count.saturating_add(1);
    }

    /// Records one bounded prompt reference.
    pub(crate) const fn add_prompt_reference(&mut self) {
        self.prompt_reference_count = self.prompt_reference_count.saturating_add(1);
    }

    /// Records one bounded conversation reference.
    pub(crate) const fn add_conversation_reference(&mut self) {
        self.conversation_reference_count = self.conversation_reference_count.saturating_add(1);
    }

    /// Records one bounded external file reference.
    pub(crate) const fn add_external_file_reference(&mut self) {
        self.external_file_reference_count = self.external_file_reference_count.saturating_add(1);
    }

    /// Records one bounded external image reference.
    pub(crate) const fn add_external_image_reference(&mut self) {
        self.external_image_reference_count = self.external_image_reference_count.saturating_add(1);
    }

    /// Records one bounded opaque item.
    pub(crate) const fn add_opaque_item(&mut self) {
        self.opaque_item_count = self.opaque_item_count.saturating_add(1);
    }
}

/// Presence observations used to derive visibility.
///
/// The request-derived strings are borrowed for one call and never retained. Most callers should
/// use [`derive_visibility`] through the Responses analyzer rather than construct this directly.
#[derive(Clone, Copy, Default)]
#[non_exhaustive]
pub struct VisibilityObservation<'request> {
    /// Whether the indexed/extracted explicit request is complete.
    pub explicit_request_complete: bool,
    /// Whether `previous_response_id` was present.
    pub uses_previous_response: bool,
    /// Decoded `previous_response_id`, when present and valid.
    pub previous_response_id: Option<&'request str>,
    /// Whether provider-held conversation state was present.
    pub uses_conversation_state: bool,
    /// Whether provider-held item references were present.
    pub uses_item_references: bool,
    /// Whether a provider-stored prompt reference was present.
    pub uses_prompt_reference: bool,
    /// Whether an external file reference was present.
    pub uses_external_files: bool,
    /// Whether an external image reference was present.
    pub uses_external_images: bool,
    /// Whether an opaque or reasoning item was present.
    pub contains_opaque_items: bool,
    /// Aggregate reference counts observed by the extractor.
    pub facts: ContextVisibilityFacts,
}

impl fmt::Debug for VisibilityObservation<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisibilityObservation")
            .field("explicit_request_complete", &self.explicit_request_complete)
            .field("uses_previous_response", &self.uses_previous_response)
            .field(
                "previous_response_id_present",
                &self.previous_response_id.is_some(),
            )
            .field("uses_conversation_state", &self.uses_conversation_state)
            .field("uses_item_references", &self.uses_item_references)
            .field("uses_prompt_reference", &self.uses_prompt_reference)
            .field("uses_external_files", &self.uses_external_files)
            .field("uses_external_images", &self.uses_external_images)
            .field("contains_opaque_items", &self.contains_opaque_items)
            .field("facts", &self.facts)
            .finish()
    }
}

/// A lookup over response ids already observed by the caller.
///
/// Implementations must be bounded and side-effect free. The analyzer calls this trait at most
/// once for the single `previous_response_id` in a request and never queries storage itself.
pub trait ObservedResponseIdLookup {
    /// Returns whether `response_id` was previously observed by the caller.
    fn contains_response_id(&self, response_id: &str) -> bool;
}

impl<T, S> ObservedResponseIdLookup for HashSet<T, S>
where
    T: Borrow<str> + Eq + Hash,
    S: BuildHasher,
{
    fn contains_response_id(&self, response_id: &str) -> bool {
        self.iter()
            .any(|candidate| candidate.borrow() == response_id)
    }
}

impl<T> ObservedResponseIdLookup for BTreeSet<T>
where
    T: Borrow<str> + Ord,
{
    fn contains_response_id(&self, response_id: &str) -> bool {
        self.iter()
            .any(|candidate| candidate.borrow() == response_id)
    }
}

impl ObservedResponseIdLookup for [&str] {
    fn contains_response_id(&self, response_id: &str) -> bool {
        self.contains(&response_id)
    }
}
impl<const N: usize> ObservedResponseIdLookup for [&str; N] {
    fn contains_response_id(&self, response_id: &str) -> bool {
        self.contains(&response_id)
    }
}

impl<Lookup> ObservedResponseIdLookup for Lookup
where
    Lookup: Fn(&str) -> bool,
{
    fn contains_response_id(&self, response_id: &str) -> bool {
        self(response_id)
    }
}

/// Derivation result for visibility and local provider-reference linkage.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct VisibilityAnalysis {
    /// Per-signal visibility. Its logical status is derived by `logical_context_status()`.
    pub visibility: ContextVisibility,
    /// Aggregate reference facts.
    pub facts: ContextVisibilityFacts,
}

/// Derives all eight visibility signals from observed request facts.
///
/// `lookup` is optional: with no lookup, the provider-state record still reports presence and a
/// digest but leaves `observed_response_id_match` absent. The logical status is always derived by
/// [`ContextVisibility::logical_context_status`], never copied from a caller.
#[must_use]
pub fn derive_visibility<Lookup>(
    observation: VisibilityObservation<'_>,
    lookup: Option<&Lookup>,
) -> VisibilityAnalysis
where
    Lookup: ObservedResponseIdLookup + ?Sized,
{
    let mut facts = observation.facts;
    let mut provider_state_reference = None;
    if let Some(response_id) = observation.previous_response_id {
        let observed_match = lookup.map(|known| known.contains_response_id(response_id));
        provider_state_reference = Some(ProviderStateReference::from_response_id(
            response_id,
            observed_match,
        ));
        if observed_match == Some(true) {
            facts.reference_resolved_locally = true;
        }
    }
    if provider_state_reference.is_some() {
        facts.provider_state_reference = provider_state_reference;
    }
    let visibility = ContextVisibility {
        explicit_request_complete: observation.explicit_request_complete,
        uses_previous_response: observation.uses_previous_response,
        uses_conversation_state: observation.uses_conversation_state,
        uses_item_references: observation.uses_item_references,
        uses_prompt_reference: observation.uses_prompt_reference,
        uses_external_files: observation.uses_external_files,
        uses_external_images: observation.uses_external_images,
        contains_opaque_items: observation.contains_opaque_items,
    };
    VisibilityAnalysis { visibility, facts }
}

/// Derives visibility when no locally observed response-id set is available.
#[must_use]
pub fn derive_visibility_without_lookup(
    observation: VisibilityObservation<'_>,
) -> VisibilityAnalysis {
    derive_visibility::<NoObservedResponseIds>(observation, None)
}

/// Empty lookup used by [`derive_visibility_without_lookup`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct NoObservedResponseIds;

impl ObservedResponseIdLookup for NoObservedResponseIds {
    fn contains_response_id(&self, _response_id: &str) -> bool {
        false
    }
}
