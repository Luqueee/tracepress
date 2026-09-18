use std::time::Duration;

use tracepress_core::SourceExecutionId;

/// The semantic contract of bytes emitted to the command consumer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OutputContract {
    AgentReadable,
    LineOrientedMachine,
    StructuredMachine,
    Unknown,
}

/// A supported command family. Expansion remains deliberately incremental.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CommandFamily {
    CargoTest,
    CargoCheck,
    CargoClippy,
    Ripgrep,
    GitStatus,
}

/// Why a command was left untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FailOpenReason {
    Unsupported,
    ShellSyntax,
}

/// The admission outcome for one command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RewriteDecision {
    Passthrough {
        family: CommandFamily,
        contract: OutputContract,
    },
    FailOpen(FailOpenReason),
}

/// Allowlisted metadata for one source execution; it intentionally has no command or output.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SourceExecutionMetadata {
    pub source_execution_id: SourceExecutionId,
    pub command_family: CommandFamily,
    pub output_contract: OutputContract,
    pub raw_stdout_bytes: u64,
    pub raw_stderr_bytes: u64,
    pub emitted_bytes: u64,
    pub exit_code: Option<i32>,
    /// Unix signal which terminated the child, when the platform reports one.
    pub termination_signal: Option<i32>,
    pub duration: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SourceOutputCandidate {
    pub candidate_stdout: Vec<u8>,
    pub candidate_stderr: Vec<u8>,
    pub raw_bytes: u64,
    pub candidate_bytes: u64,
    pub estimated_raw_tokens: u64,
    pub estimated_candidate_tokens: u64,
    pub omitted_passing_tests: u64,
    pub omitted_progress_lines: u64,
    /// Human-facing `git status` advice lines omitted without changing repository state facts.
    pub omitted_advisory_lines: u64,
    /// Match lines rewritten under a file header without omitting their content.
    pub grouped_match_lines: u64,
    /// Bytes added by the recovery hint and included in the candidate comparison.
    pub recovery_hint_bytes: u64,
    /// Wall-clock time spent constructing and evaluating the candidate.
    pub reducer_duration: Duration,
    pub applicable: bool,
    pub never_worse_accepted: bool,
}

/// Backwards-compatible Phase 5.0 name for the generalized Cargo candidate.
pub type CargoOutputCandidate = SourceOutputCandidate;
/// Backwards-compatible Phase 5.0 name for the generalized source candidate.
pub type CargoTestShadowCandidate = SourceOutputCandidate;
