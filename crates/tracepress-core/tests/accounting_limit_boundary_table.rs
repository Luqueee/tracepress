//! Exact-limit and one-over-limit table for mutable accounting resource axes.

use tracepress_core::{
    MaxCpuWorkUnits, MaxProcessingTimeMs, MaxResponseBodyBytes, ProcessingBudget,
    ProcessingBudgetAxis, ProcessingBudgetDecision, ProcessingCharge, ResponseLimitReason,
    StreamDecision, StreamingResponseBudget,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug)]
enum AccountingBoundaryKind {
    Response,
    ProcessingTime,
    CpuWork,
}

const ACCOUNTING_BOUNDARIES: [AccountingBoundaryKind; 3] = [
    AccountingBoundaryKind::Response,
    AccountingBoundaryKind::ProcessingTime,
    AccountingBoundaryKind::CpuWork,
];

#[test]
fn accounting_boundaries_accept_exact_and_classify_one_over() -> TestResult {
    // Given
    let exact = [0_u8; 8];
    let one_over = [0_u8; 9];

    // When / Then
    for boundary in ACCOUNTING_BOUNDARIES {
        match boundary {
            AccountingBoundaryKind::Response => {
                let maximum = MaxResponseBodyBytes::new(8)?;
                let mut exact_budget = StreamingResponseBudget::new(maximum);
                let mut over_budget = StreamingResponseBudget::new(maximum);
                assert_eq!(
                    exact_budget.observe_chunk(&exact)?,
                    StreamDecision::Continue { accepted_bytes: 8 }
                );
                assert_eq!(
                    over_budget.observe_chunk(&one_over)?,
                    StreamDecision::TerminateIncomplete {
                        accepted_bytes: 0,
                        rejected_chunk_bytes: 9,
                        reason: ResponseLimitReason::BodyLimit,
                    }
                );
            }
            AccountingBoundaryKind::ProcessingTime => {
                let maximum = MaxProcessingTimeMs::new(8)?;
                let maximum_work = MaxCpuWorkUnits::new(8)?;
                let mut exact_budget = ProcessingBudget::new(maximum, maximum_work);
                let mut over_budget = ProcessingBudget::new(maximum, maximum_work);
                assert_eq!(
                    exact_budget.charge(ProcessingCharge::new(8, 0)),
                    ProcessingBudgetDecision::Continue {
                        elapsed_ms: 8,
                        work_units: 0,
                    }
                );
                assert_eq!(
                    over_budget.charge(ProcessingCharge::new(9, 0)),
                    ProcessingBudgetDecision::Exhausted {
                        axis: ProcessingBudgetAxis::Time,
                        consumed_before: 0,
                        attempted: 9,
                        maximum: 8,
                    }
                );
            }
            AccountingBoundaryKind::CpuWork => {
                let maximum = MaxCpuWorkUnits::new(8)?;
                let maximum_time = MaxProcessingTimeMs::new(8)?;
                let mut exact_budget = ProcessingBudget::new(maximum_time, maximum);
                let mut over_budget = ProcessingBudget::new(maximum_time, maximum);
                assert_eq!(
                    exact_budget.charge(ProcessingCharge::new(0, 8)),
                    ProcessingBudgetDecision::Continue {
                        elapsed_ms: 0,
                        work_units: 8,
                    }
                );
                assert_eq!(
                    over_budget.charge(ProcessingCharge::new(0, 9)),
                    ProcessingBudgetDecision::Exhausted {
                        axis: ProcessingBudgetAxis::CpuWork,
                        consumed_before: 0,
                        attempted: 9,
                        maximum: 8,
                    }
                );
            }
        }
    }
    Ok(())
}
