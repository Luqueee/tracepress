//! Observable contract of token reconciliation.

use tracepress_context::{
    ContextVisibility, ReconciliationStatus, TokenEstimateAggregate, TokenReconciliation,
};
use tracepress_core::{ContextSnapshotId, UuidV7Generator};

fn snapshot_id() -> ContextSnapshotId {
    ContextSnapshotId::generate(&UuidV7Generator::new())
}

const fn explicit_visibility() -> ContextVisibility {
    ContextVisibility {
        explicit_request_complete: true,
        uses_previous_response: false,
        uses_conversation_state: false,
        uses_item_references: false,
        uses_prompt_reference: false,
        uses_external_files: false,
        uses_external_images: false,
        contains_opaque_items: false,
    }
}

#[test]
fn positive_residual_is_provider_minus_local() {
    let local = Some(80);

    let reconciliation = TokenReconciliation::reconcile(
        snapshot_id(),
        explicit_visibility(),
        local,
        Some(100),
        true,
    );

    assert_eq!(reconciliation.visible_estimated_tokens, Some(80));
    assert_eq!(reconciliation.provider_input_tokens, Some(100));
    assert_eq!(reconciliation.residual_tokens, Some(20));
    assert_eq!(
        reconciliation.comparability,
        ReconciliationStatus::ComparableApproximate
    );
}

#[test]
fn negative_residual_is_preserved() {
    let local = Some(150);

    let reconciliation = TokenReconciliation::reconcile(
        snapshot_id(),
        explicit_visibility(),
        local,
        Some(100),
        true,
    );

    assert_eq!(reconciliation.residual_tokens, Some(-50));
}

#[test]
fn zero_residual_is_known() {
    let local = Some(0);

    let reconciliation =
        TokenReconciliation::reconcile(snapshot_id(), explicit_visibility(), local, Some(0), true);

    assert_eq!(reconciliation.visible_estimated_tokens, Some(0));
    assert_eq!(reconciliation.residual_tokens, Some(0));
    assert_eq!(
        reconciliation.comparability,
        ReconciliationStatus::ComparableApproximate
    );
}

#[test]
fn missing_provider_usage_is_explicit() {
    let local = Some(80);

    let reconciliation =
        TokenReconciliation::reconcile(snapshot_id(), explicit_visibility(), local, None, true);

    assert_eq!(reconciliation.provider_input_tokens, None);
    assert_eq!(reconciliation.residual_tokens, None);
    assert_eq!(
        reconciliation.comparability,
        ReconciliationStatus::MissingProviderUsage
    );
}

#[test]
fn incomplete_local_aggregate_is_missing_not_a_subtotal() {
    let mut local = TokenEstimateAggregate::new();
    local.observe_estimate(None);

    let reconciliation = TokenReconciliation::reconcile(
        snapshot_id(),
        explicit_visibility(),
        local.complete_total(),
        Some(100),
        true,
    );

    assert_eq!(reconciliation.visible_estimated_tokens, None);
    assert_eq!(reconciliation.residual_tokens, None);
    assert_eq!(
        reconciliation.comparability,
        ReconciliationStatus::MissingLocalEstimate
    );
}

#[test]
fn provider_managed_context_has_partial_visibility() {
    let local = Some(80);
    let visibility = ContextVisibility {
        uses_previous_response: true,
        ..explicit_visibility()
    };

    let reconciliation =
        TokenReconciliation::reconcile(snapshot_id(), visibility, local, Some(100), true);

    assert_eq!(reconciliation.residual_tokens, Some(20));
    assert_eq!(
        reconciliation.comparability,
        ReconciliationStatus::PartialVisibility
    );
}

#[test]
fn external_mixed_and_unknown_contexts_have_partial_visibility() {
    let local = Some(80);
    let visibilities = [
        ContextVisibility {
            uses_external_files: true,
            ..explicit_visibility()
        },
        ContextVisibility {
            uses_conversation_state: true,
            uses_external_images: true,
            ..explicit_visibility()
        },
        ContextVisibility {
            explicit_request_complete: false,
            ..explicit_visibility()
        },
    ];

    for visibility in visibilities {
        let reconciliation =
            TokenReconciliation::reconcile(snapshot_id(), visibility, local, Some(100), true);
        assert_eq!(reconciliation.residual_tokens, Some(20));
        assert_eq!(
            reconciliation.comparability,
            ReconciliationStatus::PartialVisibility
        );
    }
}

#[test]
fn unsupported_estimator_or_incompatible_units_are_not_compared() {
    let local = Some(80);

    let reconciliation = TokenReconciliation::reconcile(
        snapshot_id(),
        explicit_visibility(),
        local,
        Some(100),
        false,
    );

    assert_eq!(reconciliation.visible_estimated_tokens, Some(80));
    assert_eq!(reconciliation.provider_input_tokens, Some(100));
    assert_eq!(reconciliation.residual_tokens, None);
    assert_eq!(
        reconciliation.comparability,
        ReconciliationStatus::NotComparable
    );
}

#[test]
fn signed_durable_boundary_is_comparable_without_saturation() {
    let maximum = u64::try_from(i64::MAX).unwrap_or(u64::MAX);
    let local = Some(maximum);

    let reconciliation =
        TokenReconciliation::reconcile(snapshot_id(), explicit_visibility(), local, Some(0), true);

    assert_eq!(reconciliation.residual_tokens, Some(-i64::MAX));
    assert_eq!(
        reconciliation.comparability,
        ReconciliationStatus::ComparableApproximate
    );
}

#[test]
fn values_above_signed_durable_range_degrade_without_overflow() {
    let above_maximum = u64::try_from(i64::MAX)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let small_local = Some(1);
    let large_local = Some(above_maximum);

    let provider_out_of_range = TokenReconciliation::reconcile(
        snapshot_id(),
        explicit_visibility(),
        small_local,
        Some(above_maximum),
        true,
    );
    let local_out_of_range = TokenReconciliation::reconcile(
        snapshot_id(),
        explicit_visibility(),
        large_local,
        Some(1),
        true,
    );

    for reconciliation in [provider_out_of_range, local_out_of_range] {
        assert_eq!(reconciliation.residual_tokens, None);
        assert_eq!(
            reconciliation.comparability,
            ReconciliationStatus::NotComparable
        );
    }
}

#[test]
fn reconciliation_copies_provider_usage_without_changing_it() {
    let local = Some(150);
    let provider_usage = Some(100);

    let reconciliation = TokenReconciliation::reconcile(
        snapshot_id(),
        explicit_visibility(),
        local,
        provider_usage,
        true,
    );

    assert_eq!(provider_usage, Some(100));
    assert_eq!(reconciliation.provider_input_tokens, provider_usage);
    assert_eq!(reconciliation.residual_tokens, Some(-50));
}
