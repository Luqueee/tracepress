//! Bounded request-body properties with deterministic generated inputs.
#![cfg(feature = "fuzz-tests")]

use proptest::{
    prelude::*,
    strategy::ValueTree,
    test_runner::{TestCaseError, TestRunner},
};
use tracepress_core::{
    ByteBoundaryError, ByteDecision, ByteResource, MaxRequestBodyBytes, classify_request_body,
};

include!("support/proptest_config.rs");

#[derive(Debug, Eq, PartialEq)]
enum ExpectedRequestDecision {
    WithinLimit,
    Reject { observed: u64 },
    Mismatch { declared: u64, actual: u64 },
}

#[test]
fn fixed_seed_replays_the_same_64_generated_cases() -> Result<(), TestCaseError> {
    // Given
    let strategy = (
        prop::collection::vec(any::<u8>(), 0..=64),
        prop::option::of(any::<u64>()),
        1_u64..=64,
    );
    let first_config = deterministic_proptest_config();
    let second_config = deterministic_proptest_config();
    assert_eq!(first_config.cases, 64);
    assert_eq!(first_config.rng_algorithm, RngAlgorithm::ChaCha);
    assert_eq!(first_config.rng_seed, RngSeed::Fixed(0x5452_4143_4550_5245));
    let mut first_runner = TestRunner::new(first_config);
    let mut second_runner = TestRunner::new(second_config);

    // When / Then
    for _case in 0..PROPTEST_CASES {
        let first = strategy
            .new_tree(&mut first_runner)
            .map_err(|error| TestCaseError::fail(error.to_string()))?
            .current();
        let second = strategy
            .new_tree(&mut second_runner)
            .map_err(|error| TestCaseError::fail(error.to_string()))?
            .current();
        assert_eq!(first, second);
    }
    Ok(())
}

proptest! {
    #![proptest_config(deterministic_proptest_config())]

    #[test]
    fn request_decision_matches_declared_length_and_actual_size_model(
        bytes in prop::collection::vec(any::<u8>(), 0..=512),
        declared in prop::option::of(any::<u64>()),
        limit in 1_u64..=512,
    ) {
        // Given
        let maximum = MaxRequestBodyBytes::new(limit)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let actual = u64::try_from(bytes.len())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let expected = match declared {
            Some(claimed) if claimed > limit => {
                ExpectedRequestDecision::Reject { observed: claimed }
            }
            None | Some(_) if actual > limit => {
                ExpectedRequestDecision::Reject { observed: actual }
            }
            Some(claimed) if claimed != actual => ExpectedRequestDecision::Mismatch {
                declared: claimed,
                actual,
            },
            None | Some(_) => ExpectedRequestDecision::WithinLimit,
        };

        // When
        let decision = classify_request_body(&bytes, declared, maximum);

        // Then
        match (expected, decision) {
            (
                ExpectedRequestDecision::Reject { observed },
                Ok(ByteDecision::Reject { violation }),
            ) => {
                prop_assert_eq!(violation.resource, ByteResource::RequestBody);
                prop_assert_eq!(violation.observed, observed);
                prop_assert_eq!(violation.maximum, limit);
            }
            (
                ExpectedRequestDecision::Mismatch {
                    declared: expected_declared,
                    actual: expected_actual,
                },
                Err(ByteBoundaryError::DeclaredLengthMismatch {
                    resource,
                    declared: actual_declared,
                    actual: actual_received,
                }),
            ) => {
                prop_assert_eq!(resource, ByteResource::RequestBody);
                prop_assert_eq!(actual_declared, expected_declared);
                prop_assert_eq!(actual_received, expected_actual);
            }
            (
                ExpectedRequestDecision::WithinLimit,
                Ok(ByteDecision::WithinLimit { bytes: observed }),
            ) => {
                prop_assert!(actual <= limit);
                prop_assert!(declared.is_none_or(|claimed| claimed == actual));
                prop_assert_eq!(observed, bytes.as_slice());
            }
            (expected, actual) => {
                return Err(TestCaseError::fail(format!(
                    "expected {expected:?}, got {actual:?}"
                )));
            }
        }
    }
}
