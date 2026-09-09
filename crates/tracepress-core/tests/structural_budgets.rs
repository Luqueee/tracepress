//! Structural metadata and deterministic work-budget contract tests.

use tracepress_core::{
    JsonBudgetAxis, JsonInspectionDecision, JsonShape, MaxCpuWorkUnits, MaxIpcQueueItems,
    MaxJsonItems, MaxJsonNesting, MaxProcessingTimeMs, ProcessingBudget, ProcessingBudgetAxis,
    ProcessingBudgetDecision, ProcessingCharge, QueueDecision, classify_ipc_queue,
    classify_json_shape,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn deep_json_shape_metadata_bypasses_without_parsing_provider_bytes() -> TestResult {
    // Given
    let shape = JsonShape::new(129, 1);

    // When
    let decision = classify_json_shape(shape, MaxJsonNesting::new(128)?, MaxJsonItems::new(1_000)?);

    // Then
    assert_eq!(
        decision,
        JsonInspectionDecision::Bypass {
            axis: JsonBudgetAxis::Nesting,
            observed: 129,
            maximum: 128,
        }
    );
    Ok(())
}

#[test]
fn json_item_budget_is_checked_independently_of_nesting() -> TestResult {
    // Given
    let shape = JsonShape::new(1, 1_001);

    // When
    let decision = classify_json_shape(shape, MaxJsonNesting::new(128)?, MaxJsonItems::new(1_000)?);

    // Then
    assert_eq!(
        decision,
        JsonInspectionDecision::Bypass {
            axis: JsonBudgetAxis::Items,
            observed: 1_001,
            maximum: 1_000,
        }
    );
    Ok(())
}

#[test]
fn queue_boundary_rejects_overflow_without_wrapping() -> TestResult {
    // Given
    let maximum = MaxIpcQueueItems::new(u64::MAX)?;

    // When
    let decision = classify_ipc_queue(u64::MAX, 1, maximum);

    // Then
    assert_eq!(
        decision,
        QueueDecision::Reject {
            current_items: u64::MAX,
            incoming_items: 1,
            maximum_items: u64::MAX,
        }
    );
    Ok(())
}

#[test]
fn processing_budget_exhaustion_uses_supplied_units_not_wall_clock() -> TestResult {
    // Given
    let mut budget =
        ProcessingBudget::new(MaxProcessingTimeMs::new(10)?, MaxCpuWorkUnits::new(100)?);

    // When
    let accepted = budget.charge(ProcessingCharge::new(10, 50));
    let exhausted = budget.charge(ProcessingCharge::new(1, 1));

    // Then
    assert_eq!(
        accepted,
        ProcessingBudgetDecision::Continue {
            elapsed_ms: 10,
            work_units: 50,
        }
    );
    assert_eq!(
        exhausted,
        ProcessingBudgetDecision::Exhausted {
            axis: ProcessingBudgetAxis::Time,
            consumed_before: 10,
            attempted: 1,
            maximum: 10,
        }
    );
    Ok(())
}

#[test]
fn cpu_work_budget_exhaustion_preserves_previous_counters() -> TestResult {
    // Given
    let mut budget =
        ProcessingBudget::new(MaxProcessingTimeMs::new(100)?, MaxCpuWorkUnits::new(10)?);

    // When
    let accepted = budget.charge(ProcessingCharge::new(1, 10));
    let exhausted = budget.charge(ProcessingCharge::new(1, 1));

    // Then
    assert_eq!(
        accepted,
        ProcessingBudgetDecision::Continue {
            elapsed_ms: 1,
            work_units: 10,
        }
    );
    assert_eq!(
        exhausted,
        ProcessingBudgetDecision::Exhausted {
            axis: ProcessingBudgetAxis::CpuWork,
            consumed_before: 10,
            attempted: 1,
            maximum: 10,
        }
    );
    Ok(())
}

#[test]
fn failed_work_charge_does_not_partially_charge_processing_time() -> TestResult {
    // Given
    let mut budget = ProcessingBudget::new(MaxProcessingTimeMs::new(10)?, MaxCpuWorkUnits::new(5)?);

    // When
    let rejected = budget.charge(ProcessingCharge::new(4, 6));
    let accepted = budget.charge(ProcessingCharge::new(10, 5));

    // Then
    assert_eq!(
        rejected,
        ProcessingBudgetDecision::Exhausted {
            axis: ProcessingBudgetAxis::CpuWork,
            consumed_before: 0,
            attempted: 6,
            maximum: 5,
        }
    );
    assert_eq!(
        accepted,
        ProcessingBudgetDecision::Continue {
            elapsed_ms: 10,
            work_units: 5,
        }
    );
    Ok(())
}
