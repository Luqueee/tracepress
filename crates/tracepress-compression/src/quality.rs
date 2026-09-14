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
    /// A bounded build command produced the expected result.
    Build,
}

/// Metadata-only family assigned to a `ToolResult` producer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum ToolFamily {
    /// Search, grep, or code-index results.
    Search,
    /// Test runner and test report results.
    Tests,
    /// Compilation and build diagnostics.
    Build,
    /// Static analysis and lint diagnostics.
    Lint,
    /// Package and dependency resolution results.
    Dependency,
    /// Version-control status, history, and diff results.
    VersionControl,
    /// File listing and filesystem inspection results.
    Filesystem,
    /// Generic shell or command execution results.
    ShellGeneric,
    /// Explicitly structured data not assigned to another family.
    StructuredData,
    /// No safe family classification was available.
    Unknown,
}

/// Transient semantic family for a generic shell invocation.
///
/// This label is derived from bounded command/output signals while a request is in memory. It is
/// safe to persist as an aggregate label, but the command, arguments, working directory, and
/// output that produced it must never cross the metadata boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum ShellSemanticFamily {
    /// Search and code-index output such as `rg` or `grep`.
    Search,
    /// Test runner output.
    Tests,
    /// Build and compilation diagnostics.
    Build,
    /// Static-analysis and lint output.
    Lint,
    /// Dependency and package-resolution output.
    Dependency,
    /// Version-control output.
    VersionControl,
    /// A shell command without a safer semantic family.
    Generic,
    /// Signals were insufficient or contradictory.
    Unknown,
}

impl ShellSemanticFamily {
    /// Stable metadata/report label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Tests => "tests",
            Self::Build => "build",
            Self::Lint => "lint",
            Self::Dependency => "dependency",
            Self::VersionControl => "version_control",
            Self::Generic => "generic",
            Self::Unknown => "unknown",
        }
    }

    /// Classify bounded transient command/output signals without retaining either input.
    #[must_use]
    pub fn from_transient_signals(command: Option<&[u8]>, output: Option<&[u8]>) -> Self {
        let command = command.map_or_else(String::new, bounded_signal);
        let output = output.map_or_else(String::new, bounded_signal);
        let classify = |signal: &str| {
            if contains_any(signal, &["rg ", "rg\"", "grep", "ripgrep", "search"])
                || signal.contains("match") && signal.contains("line_number")
            {
                Some(Self::Search)
            } else if contains_any(
                signal,
                &["cargo test", "pytest", "tox", "npm test", "vitest", "jest"],
            ) || contains_any(signal, &["passed", "failed", "skipped", "test session"])
                && (signal.contains("test")
                    || signal.contains("pytest")
                    || signal.contains("passed") && signal.contains("failed"))
            {
                Some(Self::Tests)
            } else if contains_any(signal, &["cargo build", "cargo check", "make", "compile"])
                || contains_any(signal, &["compiling", "built", "build finished"])
            {
                Some(Self::Build)
            } else if contains_any(signal, &["clippy", "ruff", "eslint", "mypy", "lint"])
                || contains_any(signal, &["warning:", "error:"]) && signal.contains("diagnostic")
            {
                Some(Self::Lint)
            } else if contains_any(
                signal,
                &[
                    "cargo tree",
                    "npm install",
                    "npm ls",
                    "pnpm",
                    "dependency",
                    "resolver",
                ],
            ) {
                Some(Self::Dependency)
            } else if contains_any(
                signal,
                &["git ", "git\"", "git status", "git diff", "git log"],
            ) {
                Some(Self::VersionControl)
            } else {
                None
            }
        };
        match (classify(&command), classify(&output)) {
            (Some(command), Some(output)) if command != output => Self::Unknown,
            (Some(family), _) | (_, Some(family)) => family,
            (None, None) if !command.is_empty() || !output.is_empty() => Self::Generic,
            (None, None) => Self::Unknown,
        }
    }
}

fn bounded_signal(bytes: &[u8]) -> String {
    const MAX_SIGNAL_BYTES: usize = 4096;
    let bounded = bytes
        .get(..bytes.len().min(MAX_SIGNAL_BYTES))
        .unwrap_or(bytes);
    String::from_utf8_lossy(bounded).to_ascii_lowercase()
}

fn contains_any(signal: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| signal.contains(marker))
}

impl ToolFamily {
    /// Classifies a bounded tool name without returning or persisting the name itself.
    #[must_use]
    pub fn from_tool_name(name: Option<&str>) -> Self {
        let Some(name) = name else {
            return Self::Unknown;
        };
        let normalized = name.to_ascii_lowercase().replace('-', "_");
        if ["git", "hg", "svn", "version_control", "diff"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::VersionControl;
        }
        if ["search", "grep", "rg", "ripgrep", "find"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::Search;
        }
        if ["test", "pytest", "vitest", "jest", "cargo_nextest"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::Tests;
        }
        if ["lint", "clippy", "eslint", "ruff", "mypy"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::Lint;
        }
        if ["build", "compile", "cargo_check", "make"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::Build;
        }
        if [
            "dependenc",
            "package",
            "npm",
            "pnpm",
            "cargo_tree",
            "resolver",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
        {
            return Self::Dependency;
        }
        if ["file", "filesystem", "directory", "ls", "tree", "glob"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::Filesystem;
        }
        if ["json", "structured", "csv", "jq"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::StructuredData;
        }
        if ["shell", "exec", "command", "run"]
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return Self::ShellGeneric;
        }
        Self::Unknown
    }

    /// Stable metadata/report label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Tests => "tests",
            Self::Build => "build",
            Self::Lint => "lint",
            Self::Dependency => "dependency",
            Self::VersionControl => "version_control",
            Self::Filesystem => "filesystem",
            Self::ShellGeneric => "shell_generic",
            Self::StructuredData => "structured_data",
            Self::Unknown => "unknown",
        }
    }
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

/// Assignment arm for a paired quality experiment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum QualityExperimentArm {
    /// Original `ToolResult` is shown to the model.
    Control,
    /// One named reducer policy is evaluated.
    Treatment,
}

/// Metadata-only assignment persisted before a quality session starts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QualityAssignment {
    /// Stable experiment identity.
    pub experiment_id: String,
    /// Stable task identity, never a prompt or path.
    pub task_id: String,
    /// Session identity used only for analysis joins.
    pub session_id: String,
    /// Randomized experiment arm.
    pub arm: QualityExperimentArm,
    /// Reducer identity for treatment sessions.
    pub reducer_id: Option<String>,
    /// Reducer version for treatment sessions.
    pub reducer_version: Option<u32>,
    /// Policy version under test.
    pub policy_version: String,
    /// Bounded assignment seed.
    pub seed: u64,
    /// Assignment probability in basis points, preserving exactness without floats.
    pub assignment_probability_basis_points: u16,
}

/// Metadata-only session metrics for quality and provider comparisons.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct QualitySessionMetrics {
    /// Objective task outcome; null means the evaluator was unavailable.
    pub task_success: Option<bool>,
    /// Provider-reported input totals.
    pub input_total: Option<u64>,
    pub cached_input: Option<u64>,
    pub uncached_input: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
    /// Agent behavior counters.
    pub turns: Option<u64>,
    pub tool_calls: Option<u64>,
    pub repeated_tool_calls: Option<u64>,
    pub retries: Option<u64>,
    pub duration_us: Option<u64>,
    pub compactions: Option<u64>,
    /// Recovery accounting.
    pub recovery_requests: Option<u64>,
    pub recovered_tokens: Option<u64>,
    pub recovery_latency_us: Option<u64>,
    /// Local representation accounting, never provider savings.
    pub gross_omitted_tokens: Option<u64>,
    pub net_observed_context_reduction: Option<u64>,
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

    #[test]
    fn tool_family_labels_and_assignment_are_metadata_only() {
        assert_eq!(ToolFamily::VersionControl.as_str(), "version_control");
        assert_eq!(
            ToolFamily::from_tool_name(Some("exec")),
            ToolFamily::ShellGeneric
        );
        let assignment = QualityAssignment {
            experiment_id: "quality-001".to_owned(),
            task_id: "search-001".to_owned(),
            session_id: "session-001".to_owned(),
            arm: QualityExperimentArm::Treatment,
            reducer_id: Some("search_projection_v1".to_owned()),
            reducer_version: Some(1),
            policy_version: "policy-1".to_owned(),
            seed: 7,
            assignment_probability_basis_points: 5_000,
        };
        let encoded = serde_json::to_vec(&assignment).unwrap_or_default();
        let text = String::from_utf8_lossy(&encoded);
        assert!(!text.contains("prompt"));
        assert!(!text.contains("path"));
        assert!(text.contains("version"));
    }

    #[test]
    fn quality_session_metrics_preserve_nulls() {
        let metrics = QualitySessionMetrics::default();
        let encoded = serde_json::to_vec(&metrics).unwrap_or_default();
        assert!(String::from_utf8_lossy(&encoded).contains(r#""uncached_input":null"#));
        assert!(String::from_utf8_lossy(&encoded).contains(r#""task_success":null"#));
    }

    #[test]
    fn shell_family_uses_transient_signals_and_fails_closed_on_conflict() {
        assert_eq!(
            ShellSemanticFamily::from_transient_signals(Some(br#"{"cmd":"rg --json foo"}"#), None),
            ShellSemanticFamily::Search
        );
        assert_eq!(
            ShellSemanticFamily::from_transient_signals(None, Some(b"4 passed, 1 failed")),
            ShellSemanticFamily::Tests
        );
        assert_eq!(
            ShellSemanticFamily::from_transient_signals(Some(b"cargo test"), Some(b"cargo build")),
            ShellSemanticFamily::Unknown
        );
        assert_eq!(
            ShellSemanticFamily::from_transient_signals(Some(b"echo hello"), None),
            ShellSemanticFamily::Generic
        );
    }
}
