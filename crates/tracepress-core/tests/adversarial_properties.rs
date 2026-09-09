//! Bounded property tests for hostile bytes and resource arithmetic.
#![cfg(feature = "fuzz-tests")]

use proptest::{prelude::*, test_runner::TestCaseError};
use tracepress_core::{
    ByteDecision, MaxCpuWorkUnits, MaxIpcQueueItems, MaxLineBytes, MaxProcessingTimeMs,
    MaxRawBytes, MaxResponseBodyBytes, ProcessingBudget, ProcessingBudgetAxis,
    ProcessingBudgetDecision, ProcessingCharge, QueueDecision, StreamDecision,
    StreamingResponseBudget, classify_ipc_queue, classify_line_bytes, classify_raw_bytes,
};

include!("support/proptest_config.rs");

proptest! {
    #![proptest_config(deterministic_proptest_config())]

    #[test]
    fn arbitrary_bytes_are_classified_without_decoding_or_mutation(
        bytes in prop::collection::vec(any::<u8>(), 0..=512),
        limit in 1_u64..=512,
    ) {
        // Given
        let maximum = MaxRawBytes::new(limit)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // When
        let decision = classify_raw_bytes(&bytes, maximum)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // Then
        match decision {
            ByteDecision::WithinLimit { bytes: observed } => {
                prop_assert!(bytes.len() <= usize::try_from(limit).unwrap_or(usize::MAX));
                prop_assert_eq!(observed, bytes.as_slice());
            }
            ByteDecision::Bypass { bytes: observed, violation } => {
                prop_assert!(bytes.len() > usize::try_from(limit).unwrap_or(usize::MAX));
                prop_assert_eq!(observed, bytes.as_slice());
                prop_assert_eq!(violation.maximum, limit);
            }
            ByteDecision::Reject { violation } => {
                return Err(TestCaseError::fail(format!("unexpected rejection: {violation:?}")));
            }
            ByteDecision::TruncateSafe { metadata, .. } => {
                return Err(TestCaseError::fail(format!("unexpected truncation: {metadata:?}")));
            }
        }
    }

    #[test]
    fn line_truncation_metadata_obeys_checked_boundary_arithmetic(
        bytes in prop::collection::vec(any::<u8>(), 0..=512),
        limit in 1_u64..=512,
    ) {
        // Given
        let maximum = MaxLineBytes::new(limit)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let original = u64::try_from(bytes.len())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // When
        let decision = classify_line_bytes(&bytes, maximum)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // Then
        match decision {
            ByteDecision::WithinLimit { bytes: observed } => {
                prop_assert!(original <= limit);
                prop_assert_eq!(observed, bytes.as_slice());
            }
            ByteDecision::TruncateSafe { bytes: observed, metadata } => {
                prop_assert!(original > limit);
                prop_assert_eq!(observed, bytes.as_slice());
                prop_assert_eq!(metadata.original_bytes, original);
                prop_assert_eq!(metadata.retained_prefix_bytes, limit);
                prop_assert_eq!(
                    metadata.retained_prefix_bytes.checked_add(metadata.omitted_bytes),
                    Some(original)
                );
            }
            other => {
                return Err(TestCaseError::fail(format!("unexpected line decision: {other:?}")));
            }
        }
    }

    #[test]
    fn streaming_accounting_never_accepts_more_than_the_hard_limit(
        chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..=64), 0..=32),
        limit in 1_u64..=2_048,
    ) {
        // Given
        let maximum = MaxResponseBodyBytes::new(limit)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut budget = StreamingResponseBudget::new(maximum);

        // When
        for chunk in chunks {
            let decision = budget.observe_chunk(&chunk)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            match decision {
                StreamDecision::Continue { accepted_bytes } => {
                    prop_assert!(accepted_bytes <= limit);
                }
                StreamDecision::TerminateIncomplete { accepted_bytes, .. } => {
                    prop_assert!(accepted_bytes <= limit);
                    break;
                }
            }
        }

        // Then
        prop_assert!(budget.accepted_bytes() <= limit);
    }

    #[test]
    fn queue_admission_matches_independent_checked_addition(
        current_items in any::<u64>(),
        incoming_items in any::<u64>(),
        limit in 1_u64..=u64::MAX,
    ) {
        // Given
        let maximum = MaxIpcQueueItems::new(limit)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // When
        let decision = classify_ipc_queue(current_items, incoming_items, maximum);

        // Then
        match current_items.checked_add(incoming_items) {
            Some(total_items) if total_items <= limit => {
                prop_assert_eq!(decision, QueueDecision::WithinLimit { total_items });
            }
            Some(_) | None => {
                prop_assert_eq!(
                    decision,
                    QueueDecision::Reject {
                        current_items,
                        incoming_items,
                        maximum_items: limit,
                    }
                );
            }
        }
    }

    #[test]
    fn processing_charges_match_independent_checked_counter_model(
        charges in prop::collection::vec((any::<u64>(), any::<u64>()), 0..=16),
        time_limit in 1_u64..=u64::MAX,
        work_limit in 1_u64..=u64::MAX,
    ) {
        // Given
        let maximum_time = MaxProcessingTimeMs::new(time_limit)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let maximum_work = MaxCpuWorkUnits::new(work_limit)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut budget = ProcessingBudget::new(maximum_time, maximum_work);
        let mut elapsed_ms = 0_u64;
        let mut work_units = 0_u64;

        // When
        for (time_charge, work_charge) in charges {
            let decision = budget.charge(ProcessingCharge::new(time_charge, work_charge));

            // Then
            let Some(next_time) = elapsed_ms.checked_add(time_charge) else {
                prop_assert_eq!(
                    decision,
                    ProcessingBudgetDecision::Exhausted {
                        axis: ProcessingBudgetAxis::Time,
                        consumed_before: elapsed_ms,
                        attempted: time_charge,
                        maximum: time_limit,
                    }
                );
                continue;
            };
            if next_time > time_limit {
                prop_assert_eq!(
                    decision,
                    ProcessingBudgetDecision::Exhausted {
                        axis: ProcessingBudgetAxis::Time,
                        consumed_before: elapsed_ms,
                        attempted: time_charge,
                        maximum: time_limit,
                    }
                );
                continue;
            }
            let Some(next_work) = work_units.checked_add(work_charge) else {
                prop_assert_eq!(
                    decision,
                    ProcessingBudgetDecision::Exhausted {
                        axis: ProcessingBudgetAxis::CpuWork,
                        consumed_before: work_units,
                        attempted: work_charge,
                        maximum: work_limit,
                    }
                );
                continue;
            };
            if next_work > work_limit {
                prop_assert_eq!(
                    decision,
                    ProcessingBudgetDecision::Exhausted {
                        axis: ProcessingBudgetAxis::CpuWork,
                        consumed_before: work_units,
                        attempted: work_charge,
                        maximum: work_limit,
                    }
                );
                continue;
            }
            elapsed_ms = next_time;
            work_units = next_work;
            prop_assert_eq!(
                decision,
                ProcessingBudgetDecision::Continue {
                    elapsed_ms,
                    work_units,
                }
            );
        }
    }
}
