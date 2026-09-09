//! Exact-limit and one-over-limit table for structural resource axes.

use tracepress_core::{
    JsonBudgetAxis, JsonInspectionDecision, JsonShape, MaxIpcQueueItems, MaxJsonItems,
    MaxJsonNesting, QueueDecision, classify_ipc_queue, classify_json_shape,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug)]
enum StructuralBoundaryKind {
    JsonNesting,
    JsonItems,
    Queue,
}

const STRUCTURAL_BOUNDARIES: [StructuralBoundaryKind; 3] = [
    StructuralBoundaryKind::JsonNesting,
    StructuralBoundaryKind::JsonItems,
    StructuralBoundaryKind::Queue,
];

#[test]
fn structural_boundaries_accept_exact_and_classify_one_over() -> TestResult {
    // Given / When / Then
    for boundary in STRUCTURAL_BOUNDARIES {
        match boundary {
            StructuralBoundaryKind::JsonNesting => {
                let maximum = MaxJsonNesting::new(8)?;
                let maximum_items = MaxJsonItems::new(8)?;
                assert_eq!(
                    classify_json_shape(JsonShape::new(8, 1), maximum, maximum_items),
                    JsonInspectionDecision::Inspect
                );
                assert_eq!(
                    classify_json_shape(JsonShape::new(9, 1), maximum, maximum_items),
                    JsonInspectionDecision::Bypass {
                        axis: JsonBudgetAxis::Nesting,
                        observed: 9,
                        maximum: 8,
                    }
                );
            }
            StructuralBoundaryKind::JsonItems => {
                let maximum = MaxJsonItems::new(8)?;
                let maximum_nesting = MaxJsonNesting::new(8)?;
                assert_eq!(
                    classify_json_shape(JsonShape::new(1, 8), maximum_nesting, maximum),
                    JsonInspectionDecision::Inspect
                );
                assert_eq!(
                    classify_json_shape(JsonShape::new(1, 9), maximum_nesting, maximum),
                    JsonInspectionDecision::Bypass {
                        axis: JsonBudgetAxis::Items,
                        observed: 9,
                        maximum: 8,
                    }
                );
            }
            StructuralBoundaryKind::Queue => {
                let maximum = MaxIpcQueueItems::new(8)?;
                assert_eq!(
                    classify_ipc_queue(7, 1, maximum),
                    QueueDecision::WithinLimit { total_items: 8 }
                );
                assert_eq!(
                    classify_ipc_queue(7, 2, maximum),
                    QueueDecision::Reject {
                        current_items: 7,
                        incoming_items: 2,
                        maximum_items: 8,
                    }
                );
            }
        }
    }
    Ok(())
}
