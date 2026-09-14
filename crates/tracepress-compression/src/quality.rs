//! Objective, metadata-only quality contracts for Phase 4.3 pilots.
//!
//! This module describes verifiable task outcomes without carrying prompts, tool output, source
//! files, or evaluator command lines. Execution remains outside this crate and must fail open at
//! the forwarding boundary.

use serde::{Deserialize, Serialize};

/// Objective evaluator selected for a quality task.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum QualityEvaluatorKind {
    /// The task process returned the expected exit status.
    ExitCode,
    /// A bounded test suite produced the expected pass/fail result.
    TestSuite,
    /// A declared artifact changed (or stayed unchanged) as expected.
    FileChange,
    /// A metadata-only answer key matched the observed result.
    KnownAnswer,
}

/// Expected artifact category, without a path or file contents.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum ExpectedArtifactKind {
    /// No artifact is required.
    None,
    /// A test report or test result exists.
    TestReport,
    /// A build result exists.
    BuildResult,
    /// A bounded repository query result exists.
    QueryResult,
}

/// Metadata-only description of a reproducible quality task.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QualityTask {
    /// Stable task identifier, never a prompt or repository path.
    pub task_id: String,
    /// Coarse workload class such as tests, build, or search.
    pub workload_class: String,
    /// Objective evaluator used for the task.
    pub evaluator: QualityEvaluatorKind,
    /// Maximum evaluator duration in milliseconds.
    pub timeout_ms: u64,
    /// Expected artifact category, when applicable.
    pub expected_artifact: ExpectedArtifactKind,
}

/// Terminal quality state from an objective evaluator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum QualityOutcomeStatus {
    /// The objective task completed successfully.
    Success,
    /// The objective task completed but did not satisfy its gate.
    Failure,
    /// The task could not be evaluated within its declared boundary.
    Unavailable,
}

/// Metadata-only outcome for one task execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QualityOutcome {
    /// Task identity.
    pub task_id: String,
    /// Terminal status.
    pub status: QualityOutcomeStatus,
    /// Exit status when an external task provided one.
    pub exit_code: Option<i32>,
    /// Number of passed tests, when the evaluator provides it.
    pub tests_passed: Option<u64>,
    /// Number of failed tests, when the evaluator provides it.
    pub tests_failed: Option<u64>,
    /// Whether the expected artifact condition held.
    pub artifact_match: Option<bool>,
    /// Evaluator duration in microseconds.
    pub duration_us: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_contract_roundtrips_without_content_fields() {
        let task = QualityTask {
            task_id: "task-001".to_owned(),
            workload_class: "tests".to_owned(),
            evaluator: QualityEvaluatorKind::TestSuite,
            timeout_ms: 30_000,
            expected_artifact: ExpectedArtifactKind::TestReport,
        };
        let encoded_result = serde_json::to_vec(&task);
        assert!(encoded_result.is_ok());
        let encoded = encoded_result.unwrap_or_default();
        let decoded_result: Result<QualityTask, _> = serde_json::from_slice(&encoded);
        assert!(decoded_result.is_ok());
        let decoded = decoded_result.unwrap_or_else(|_| task.clone());
        assert_eq!(decoded, task);
        assert!(!String::from_utf8_lossy(&encoded).contains("prompt"));
        assert!(!String::from_utf8_lossy(&encoded).contains("content"));
    }

    #[test]
    fn unavailable_metrics_remain_null() {
        let outcome = QualityOutcome {
            task_id: "task-001".to_owned(),
            status: QualityOutcomeStatus::Unavailable,
            exit_code: None,
            tests_passed: None,
            tests_failed: None,
            artifact_match: None,
            duration_us: None,
        };
        let encoded_result = serde_json::to_vec(&outcome);
        assert!(encoded_result.is_ok());
        let encoded = encoded_result.unwrap_or_default();
        assert!(String::from_utf8_lossy(&encoded).contains(r#""exit_code":null"#));
        assert!(String::from_utf8_lossy(&encoded).contains(r#""tests_passed":null"#));
    }
}
