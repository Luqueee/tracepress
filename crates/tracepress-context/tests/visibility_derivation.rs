//! Visibility-to-logical-status derivation contract tests.

use tracepress_context::{ContextVisibility, LogicalContextStatus};

type SignalSetter = fn(&mut ContextVisibility);

const PROVIDER_MANAGED_SIGNALS: [(&str, SignalSetter); 5] = [
    ("previous_response", |visibility| {
        visibility.uses_previous_response = true;
    }),
    ("conversation_state", |visibility| {
        visibility.uses_conversation_state = true;
    }),
    ("item_references", |visibility| {
        visibility.uses_item_references = true;
    }),
    ("prompt_reference", |visibility| {
        visibility.uses_prompt_reference = true;
    }),
    ("opaque_items", |visibility| {
        visibility.contains_opaque_items = true;
    }),
];

const EXTERNAL_SIGNALS: [(&str, SignalSetter); 2] = [
    ("external_files", |visibility| {
        visibility.uses_external_files = true;
    }),
    ("external_images", |visibility| {
        visibility.uses_external_images = true;
    }),
];

#[test]
fn a_completely_observed_request_without_signals_is_explicit_only() {
    // Given
    let visibility = completely_observed();

    // When
    let status = visibility.logical_context_status();

    // Then
    assert_eq!(status, LogicalContextStatus::ExplicitOnly);
}

#[test]
fn an_incompletely_observed_request_without_signals_is_unknown() {
    // Given
    let mut visibility = completely_observed();
    visibility.explicit_request_complete = false;

    // When
    let status = visibility.logical_context_status();

    // Then
    assert_eq!(status, LogicalContextStatus::Unknown);
}

#[test]
fn every_provider_managed_signal_yields_provider_managed_partial() {
    for (signal, set_signal) in PROVIDER_MANAGED_SIGNALS {
        // Given
        let mut visibility = completely_observed();
        set_signal(&mut visibility);

        // When
        let status = visibility.logical_context_status();

        // Then
        assert_eq!(
            status,
            LogicalContextStatus::ProviderManagedPartial,
            "signal {signal} must not be reported as explicit-only"
        );
    }
}

#[test]
fn every_external_signal_yields_external_references_partial() {
    for (signal, set_signal) in EXTERNAL_SIGNALS {
        // Given
        let mut visibility = completely_observed();
        set_signal(&mut visibility);

        // When
        let status = visibility.logical_context_status();

        // Then
        assert_eq!(
            status,
            LogicalContextStatus::ExternalReferencesPartial,
            "signal {signal} must not be reported as explicit-only"
        );
    }
}

#[test]
fn several_provider_managed_signals_stay_provider_managed_partial() {
    // Given
    let mut visibility = completely_observed();
    visibility.uses_previous_response = true;
    visibility.uses_item_references = true;
    visibility.contains_opaque_items = true;

    // When
    let status = visibility.logical_context_status();

    // Then
    assert_eq!(status, LogicalContextStatus::ProviderManagedPartial);
}

#[test]
fn signals_from_both_classes_yield_mixed_partial() {
    for (provider_signal, set_provider_signal) in PROVIDER_MANAGED_SIGNALS {
        for (external_signal, set_external_signal) in EXTERNAL_SIGNALS {
            // Given
            let mut visibility = completely_observed();
            set_provider_signal(&mut visibility);
            set_external_signal(&mut visibility);

            // When
            let status = visibility.logical_context_status();

            // Then
            assert_eq!(
                status,
                LogicalContextStatus::MixedPartial,
                "signals {provider_signal} and {external_signal} span both hidden-content classes"
            );
        }
    }
}

#[test]
fn an_observed_signal_keeps_its_partial_status_when_observation_was_incomplete() {
    // Given
    let mut visibility = completely_observed();
    visibility.explicit_request_complete = false;
    visibility.uses_external_files = true;

    // When
    let status = visibility.logical_context_status();

    // Then
    assert_eq!(status, LogicalContextStatus::ExternalReferencesPartial);
}

#[test]
fn a_transported_status_never_overrides_the_derived_one() -> Result<(), serde_json::Error> {
    // Given
    let transported = r#"{
        "explicit_request_complete": true,
        "uses_previous_response": true,
        "uses_conversation_state": false,
        "uses_item_references": false,
        "uses_prompt_reference": false,
        "uses_external_files": false,
        "uses_external_images": false,
        "contains_opaque_items": false,
        "logical_context_status": "explicit_only"
    }"#;

    // When
    let visibility = serde_json::from_str::<ContextVisibility>(transported)?;

    // Then
    assert_eq!(
        visibility.logical_context_status(),
        LogicalContextStatus::ProviderManagedPartial
    );
    Ok(())
}

const fn completely_observed() -> ContextVisibility {
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
