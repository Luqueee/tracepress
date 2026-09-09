//! Structured visibility of one observed request and its logical context status.

use serde::{Deserialize, Serialize};

/// How much of the effective context Tracepress can claim to have observed.
///
/// No member claims a complete context: Tracepress knows the explicit bytes it observed, and a
/// provider-managed or external reference means the model was given content those bytes do not
/// contain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogicalContextStatus {
    /// The observed request declared no provider-managed, external, or opaque content.
    ExplicitOnly,
    /// Part of the effective context is managed by the provider rather than sent explicitly.
    ProviderManagedPartial,
    /// Part of the effective context is an external file or image reference.
    ExternalReferencesPartial,
    /// Both provider-managed and external content are referenced.
    MixedPartial,
    /// Visibility could not be determined from what was observed.
    Unknown,
}

/// Explicit per-signal visibility of one observed request.
///
/// There is deliberately no `full_context` boolean anywhere: a single flag would have to mean
/// "the model saw exactly these bytes", which Tracepress cannot know. Each signal below is an
/// independent observation, and [`ContextVisibility::logical_context_status`] derives the claim.
#[allow(
    clippy::exhaustive_structs,
    reason = "the contract fixes the signal set; a new signal must break every consumer that classifies visibility"
)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each independently observable visibility signal must remain separately queryable"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextVisibility {
    /// Every explicit byte of the request was observed and decomposed.
    pub explicit_request_complete: bool,
    /// The request declared a `previous_response_id`.
    pub uses_previous_response: bool,
    /// The request declared provider-held conversation state.
    pub uses_conversation_state: bool,
    /// The request referenced provider-held items instead of inlining them.
    pub uses_item_references: bool,
    /// The request referenced a provider-stored prompt.
    pub uses_prompt_reference: bool,
    /// The request referenced an external file.
    pub uses_external_files: bool,
    /// The request referenced an external image.
    pub uses_external_images: bool,
    /// The request carried an item whose content this analysis cannot read.
    pub contains_opaque_items: bool,
}

impl ContextVisibility {
    /// Derives the logical context status from the observed signals.
    ///
    /// The rule has three parts. Every signal implies exactly one class of hidden content —
    /// provider-managed or external — and one class present selects that class's partial status,
    /// while both classes select [`LogicalContextStatus::MixedPartial`]. With no class present,
    /// the claim still depends on whether the explicit bytes were completely observed: an
    /// incomplete analysis cannot prove
    /// the absence of a signal it never reached, so it reports
    /// [`LogicalContextStatus::Unknown`] rather than [`LogicalContextStatus::ExplicitOnly`].
    /// A signal that *was* observed is positive evidence and keeps its partial status even when
    /// the rest of the request was not decomposed.
    ///
    /// An opaque item is classed as provider-managed: its bytes are explicit and counted, while
    /// its meaning is readable only by the provider that produced it.
    #[must_use]
    pub const fn logical_context_status(self) -> LogicalContextStatus {
        match (
            self.references_provider_managed_content(),
            self.references_external_content(),
        ) {
            (true, true) => LogicalContextStatus::MixedPartial,
            (true, false) => LogicalContextStatus::ProviderManagedPartial,
            (false, true) => LogicalContextStatus::ExternalReferencesPartial,
            (false, false) if self.explicit_request_complete => LogicalContextStatus::ExplicitOnly,
            (false, false) => LogicalContextStatus::Unknown,
        }
    }

    /// Returns whether any signal implies content the provider holds or can decode.
    #[must_use]
    pub const fn references_provider_managed_content(self) -> bool {
        self.uses_previous_response
            || self.uses_conversation_state
            || self.uses_item_references
            || self.uses_prompt_reference
            || self.contains_opaque_items
    }

    /// Returns whether any signal implies content behind an unfetched external reference.
    #[must_use]
    pub const fn references_external_content(self) -> bool {
        self.uses_external_files || self.uses_external_images
    }
}
