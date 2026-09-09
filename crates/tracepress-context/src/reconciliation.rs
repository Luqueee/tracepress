//! Token reconciliation between local estimates and provider-observed usage.
//!
//! Provider usage remains truth. Reconciliation records only whether a local estimate can be
//! compared with it and, when it can, the signed difference between the two sources.

use serde::{Deserialize, Serialize};
use tracepress_core::ContextSnapshotId;

use crate::{ContextVisibility, LogicalContextStatus};

/// How honestly a provider input total can be compared with a local visible-context estimate.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReconciliationStatus {
    /// Both totals are present, representable, and approximately comparable over explicit context.
    ComparableApproximate,
    /// Both totals are present and representable, but some logical context is not locally visible.
    PartialVisibility,
    /// The provider did not report an input-token total.
    MissingProviderUsage,
    /// The local aggregate has no complete token estimate.
    MissingLocalEstimate,
    /// The totals use incompatible units or otherwise cannot be compared safely.
    NotComparable,
}

/// A bounded comparison of provider-observed input tokens with a local visible-context estimate.
///
/// The provider value is copied unchanged. [`Self::residual_tokens`] is present only when both
/// operands fit the durable signed range and the caller confirms that their units are otherwise
/// comparable. A residual is evidence about estimator accuracy and visibility, never a count of
/// hidden tokens and never billing truth.
#[allow(
    clippy::exhaustive_structs,
    reason = "the frozen durable reconciliation record fixes these fields"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TokenReconciliation {
    /// Snapshot whose local estimate is being reconciled.
    pub snapshot_id: ContextSnapshotId,
    /// Complete locally estimated total for visible context, when available and representable.
    pub visible_estimated_tokens: Option<u64>,
    /// Provider-observed input total, copied without modification.
    pub provider_input_tokens: Option<u64>,
    /// Provider total minus local total, signed and never clamped.
    pub residual_tokens: Option<i64>,
    /// Whether and why the two sources can be compared.
    pub comparability: ReconciliationStatus,
}

impl TokenReconciliation {
    /// Reconciles a local aggregate's complete total with provider-observed input usage.
    ///
    /// Pass [`crate::TokenEstimateAggregate::complete_total`] as `visible_estimated_tokens`; an
    /// incomplete aggregate therefore remains unknown instead of exposing its subtotal as a total.
    /// `otherwise_comparable` is false for unsupported estimators, incompatible token units, or
    /// any other condition that makes subtraction misleading. Missing inputs take precedence over
    /// that flag so absence remains explicit. Values above [`i64::MAX`] are retained as their
    /// original unsigned totals but produce [`ReconciliationStatus::NotComparable`] rather than an
    /// overflowing or saturated residual.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the frozen reconciliation contract has five independent inputs"
    )]
    pub fn reconcile(
        snapshot_id: ContextSnapshotId,
        visibility: ContextVisibility,
        visible_estimated_tokens: Option<u64>,
        provider_input_tokens: Option<u64>,
        otherwise_comparable: bool,
    ) -> Self {
        let (residual_tokens, comparability) =
            match (provider_input_tokens, visible_estimated_tokens) {
                (None, _) => (None, ReconciliationStatus::MissingProviderUsage),
                (Some(_), None) => (None, ReconciliationStatus::MissingLocalEstimate),
                (Some(provider), Some(local)) => {
                    let signed = i64::try_from(provider).ok().zip(i64::try_from(local).ok());
                    match signed {
                        None => (None, ReconciliationStatus::NotComparable),
                        Some(_) if !otherwise_comparable => {
                            (None, ReconciliationStatus::NotComparable)
                        }
                        Some((provider, local)) => {
                            let residual = provider.checked_sub(local);
                            let status = match visibility.logical_context_status() {
                                LogicalContextStatus::ExplicitOnly => {
                                    ReconciliationStatus::ComparableApproximate
                                }
                                LogicalContextStatus::ProviderManagedPartial
                                | LogicalContextStatus::ExternalReferencesPartial
                                | LogicalContextStatus::MixedPartial
                                | LogicalContextStatus::Unknown => {
                                    ReconciliationStatus::PartialVisibility
                                }
                            };
                            (residual, status)
                        }
                    }
                }
            };

        Self {
            snapshot_id,
            visible_estimated_tokens,
            provider_input_tokens,
            residual_tokens,
            comparability,
        }
    }
}
