//! Conservative source-side command admission and passthrough execution.
#![allow(
    missing_docs,
    reason = "Phase 5.0 internal proxy API is deliberately narrow while its runtime contract settles"
)]

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt as _;
use std::{
    process::Command,
    time::{Duration, Instant},
};

use tracepress_core::{SourceExecutionId, UuidV7Generator};

/// Builds a fail-open Codex `PreToolUse` rewrite response without granting permission.
///
/// Malformed, non-Bash, unsupported, or syntactically unsafe calls return `None`, which means
/// the hook writes nothing and Codex executes the original command.
#[must_use]
pub fn codex_pre_tool_use_rewrite(input: &[u8]) -> Option<Vec<u8>> {
    let payload: serde_json::Value = serde_json::from_slice(input).ok()?;
    if payload.get("hook_event_name")?.as_str()? != "PreToolUse"
        || payload.get("tool_name")?.as_str()? != "Bash"
    {
        return None;
    }
    let command = payload.pointer("/tool_input/command")?.as_str()?;
    let RewriteDecision::Passthrough { .. } = decide(command) else {
        return None;
    };
    let executable = std::env::var("TRACEPRESS_TOOL_BIN")
        .ok()
        .filter(|value| !value.is_empty() && !value.contains([' ', '\'', '"', '$', '`', '\\']))
        .unwrap_or_else(|| "tracepress".to_owned());
    let session = payload
        .get("session_id")
        .or_else(|| payload.get("turn_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| {
            value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    let session_prefix = session.map_or_else(String::new, |value| {
        format!("TRACEPRESS_SOURCE_SESSION_ID={value} ")
    });
    serde_json::to_vec(&serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": { "command": format!("{session_prefix}{executable} tool {command}") }
        }
    }))
    .ok()
}

/// Builds an admitted Codex rewrite that preserves the original command text.
///
/// This exists solely for Phase 5.0 attribution: it isolates the presence of a
/// `PreToolUse` `updatedInput` response from the source-tool wrapper.
#[must_use]
pub fn codex_pre_tool_use_identity_rewrite(input: &[u8]) -> Option<Vec<u8>> {
    let payload: serde_json::Value = serde_json::from_slice(input).ok()?;
    if payload.get("hook_event_name")?.as_str()? != "PreToolUse"
        || payload.get("tool_name")?.as_str()? != "Bash"
    {
        return None;
    }
    let command = payload.pointer("/tool_input/command")?.as_str()?;
    let RewriteDecision::Passthrough { .. } = decide(command) else {
        return None;
    };
    serde_json::to_vec(&serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": { "command": command }
        }
    }))
    .ok()
}

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
}

/// Why a command was left untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FailOpenReason {
    Unsupported,
    ShellSyntax,
}

/// The only admission outcome available before reducers exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RewriteDecision {
    Passthrough {
        family: CommandFamily,
        contract: OutputContract,
    },
    FailOpen(FailOpenReason),
}

/// Allowlist parser: shell composition is never split or reinterpreted.
#[must_use]
pub fn decide(command: &str) -> RewriteDecision {
    if command.contains([
        '|', '>', '<', ';', '&', '$', '`', '(', ')', '\n', '\r', '\'', '"', '\\', '*', '?', '[',
        ']', '{', '}', '~', '!',
    ]) {
        return RewriteDecision::FailOpen(FailOpenReason::ShellSyntax);
    }
    let words: Vec<_> = command.split_ascii_whitespace().collect();
    match words.as_slice() {
        ["cargo", "test", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::CargoTest,
            contract: OutputContract::AgentReadable,
        },
        ["cargo", "check", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::CargoCheck,
            contract: OutputContract::AgentReadable,
        },
        ["cargo", "clippy", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::CargoClippy,
            contract: OutputContract::AgentReadable,
        },
        ["rg", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::Ripgrep,
            contract: OutputContract::AgentReadable,
        },
        _ => RewriteDecision::FailOpen(FailOpenReason::Unsupported),
    }
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

const fn estimated_tokens(bytes: u64) -> u64 {
    bytes.saturating_add(3) / 4
}

fn cargo_test_v1_streams(stdout: &str, stderr: &str) -> (Vec<u8>, Vec<u8>, u64, u64) {
    let mut candidate_stdout = Vec::new();
    let mut omitted_passing_tests = 0_u64;
    for line in stdout.split_inclusive('\n') {
        let trimmed = line.trim();
        if (trimmed.starts_with("test ") && trimmed.ends_with(" ... ok"))
            || (trimmed.starts_with("running ") && trimmed.ends_with(" tests"))
        {
            if trimmed.starts_with("test ") {
                omitted_passing_tests = omitted_passing_tests.saturating_add(1);
            }
            continue;
        }
        candidate_stdout.extend_from_slice(line.as_bytes());
    }
    let mut candidate_stderr = Vec::new();
    let mut omitted_progress_lines = 0_u64;
    for line in stderr.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if [
            "Compiling ",
            "Checking ",
            "Downloading ",
            "Downloaded ",
            "Finished ",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
        {
            omitted_progress_lines = omitted_progress_lines.saturating_add(1);
        } else {
            candidate_stderr.extend_from_slice(line.as_bytes());
        }
    }
    (
        candidate_stdout,
        candidate_stderr,
        omitted_passing_tests,
        omitted_progress_lines,
    )
}

fn cargo_test_v1_candidate(
    stdout: &[u8],
    stderr: &[u8],
    active_recovery_command: Option<&str>,
) -> CargoOutputCandidate {
    let started = Instant::now();
    let raw_bytes = u64::try_from(stdout.len().saturating_add(stderr.len())).unwrap_or(u64::MAX);
    let (mut candidate_stdout, candidate_stderr, omitted_passing_tests, omitted_progress_lines) =
        match (std::str::from_utf8(stdout), std::str::from_utf8(stderr)) {
            (Ok(stdout), Ok(stderr)) => cargo_test_v1_streams(stdout, stderr),
            _ => (stdout.to_vec(), stderr.to_vec(), 0, 0),
        };
    let applicable = omitted_passing_tests > 0 || omitted_progress_lines > 0;
    let recovery_hint_bytes = if applicable {
        let hint = active_recovery_command.map_or_else(
            || {
                format!(
                    "[Tracepress: {omitted_passing_tests} passing test lines and {omitted_progress_lines} progress lines omitted; recovery available when active.]\n"
                )
            },
            |command| {
                format!(
                    "[Tracepress: {omitted_passing_tests} passing test lines and {omitted_progress_lines} progress lines omitted. Full output: {command}]\n"
                )
            },
        );
        let hint_bytes = u64::try_from(hint.len()).unwrap_or(u64::MAX);
        let mut prefixed = hint.into_bytes();
        prefixed.append(&mut candidate_stdout);
        candidate_stdout = prefixed;
        hint_bytes
    } else {
        0
    };
    let candidate_bytes = u64::try_from(
        candidate_stdout
            .len()
            .saturating_add(candidate_stderr.len()),
    )
    .unwrap_or(u64::MAX);
    let estimated_raw_tokens = estimated_tokens(raw_bytes);
    let estimated_candidate_tokens = estimated_tokens(candidate_bytes);
    let never_worse_accepted = applicable
        && candidate_bytes < raw_bytes
        && estimated_candidate_tokens < estimated_raw_tokens;
    CargoOutputCandidate {
        candidate_stdout,
        candidate_stderr,
        raw_bytes,
        candidate_bytes,
        estimated_raw_tokens,
        estimated_candidate_tokens,
        omitted_passing_tests,
        omitted_progress_lines,
        grouped_match_lines: 0,
        recovery_hint_bytes,
        reducer_duration: started.elapsed(),
        applicable,
        never_worse_accepted,
    }
}

/// Evaluates `cargo_test_v1` without changing agent-visible bytes.
#[must_use]
pub fn cargo_test_v1_shadow(stdout: &[u8], stderr: &[u8]) -> CargoOutputCandidate {
    cargo_test_v1_candidate(stdout, stderr, None)
}

/// Evaluates `cargo_test_v1` with the exact recovery command included in never-worse.
#[must_use]
pub fn cargo_test_v1_active(
    stdout: &[u8],
    stderr: &[u8],
    recovery_command: &str,
) -> CargoOutputCandidate {
    cargo_test_v1_candidate(stdout, stderr, Some(recovery_command))
}

#[derive(Clone, Copy)]
struct CargoCheckPolicy<'a> {
    active_recovery_command: Option<&'a str>,
    preserve_finished: bool,
}

fn cargo_check_candidate(
    stdout: &[u8],
    stderr: &[u8],
    policy: CargoCheckPolicy<'_>,
) -> CargoOutputCandidate {
    let started = Instant::now();
    let raw_bytes = u64::try_from(stdout.len().saturating_add(stderr.len())).unwrap_or(u64::MAX);
    let (candidate_stderr, omitted_progress_lines, applicable) =
        std::str::from_utf8(stderr).ok().map_or_else(
            || (stderr.to_vec(), 0, false),
            |stderr| {
                let mut candidate = Vec::new();
                let mut omitted = 0_u64;
                for line in stderr.split_inclusive('\n') {
                    let trimmed = line.trim_start();
                    if ["Compiling ", "Checking ", "Downloading ", "Downloaded "]
                        .iter()
                        .any(|prefix| trimmed.starts_with(prefix))
                        || (!policy.preserve_finished && trimmed.starts_with("Finished "))
                    {
                        omitted = omitted.saturating_add(1);
                    } else {
                        candidate.extend_from_slice(line.as_bytes());
                    }
                }
                (candidate, omitted, omitted > 0)
            },
        );
    let mut candidate_stdout = stdout.to_vec();
    let recovery_hint_bytes = if applicable {
        let hint = policy.active_recovery_command.map_or_else(
            || {
                format!(
                    "[Tracepress: {omitted_progress_lines} Cargo progress lines omitted; recovery available when active.]\n"
                )
            },
            |command| {
                format!(
                    "[Tracepress: {omitted_progress_lines} Cargo progress lines omitted. Full output: {command}]\n"
                )
            },
        );
        let hint_bytes = u64::try_from(hint.len()).unwrap_or(u64::MAX);
        let mut prefixed = hint.into_bytes();
        prefixed.append(&mut candidate_stdout);
        candidate_stdout = prefixed;
        hint_bytes
    } else {
        0
    };
    let candidate_bytes = u64::try_from(
        candidate_stdout
            .len()
            .saturating_add(candidate_stderr.len()),
    )
    .unwrap_or(u64::MAX);
    let estimated_raw_tokens = estimated_tokens(raw_bytes);
    let estimated_candidate_tokens = estimated_tokens(candidate_bytes);
    let never_worse_accepted = applicable
        && candidate_bytes < raw_bytes
        && estimated_candidate_tokens < estimated_raw_tokens;
    CargoOutputCandidate {
        candidate_stdout,
        candidate_stderr,
        raw_bytes,
        candidate_bytes,
        estimated_raw_tokens,
        estimated_candidate_tokens,
        omitted_passing_tests: 0,
        omitted_progress_lines,
        grouped_match_lines: 0,
        recovery_hint_bytes,
        reducer_duration: started.elapsed(),
        applicable,
        never_worse_accepted,
    }
}

/// Evaluates a conservative `cargo check` progress projection without changing visible bytes.
#[must_use]
pub fn cargo_check_v1_shadow(stdout: &[u8], stderr: &[u8]) -> CargoOutputCandidate {
    cargo_check_candidate(
        stdout,
        stderr,
        CargoCheckPolicy {
            active_recovery_command: None,
            preserve_finished: false,
        },
    )
}

/// Evaluates `cargo_check_v1` with the exact recovery command included in never-worse.
#[must_use]
pub fn cargo_check_v1_active(
    stdout: &[u8],
    stderr: &[u8],
    recovery_command: &str,
) -> CargoOutputCandidate {
    cargo_check_candidate(
        stdout,
        stderr,
        CargoCheckPolicy {
            active_recovery_command: Some(recovery_command),
            preserve_finished: false,
        },
    )
}

/// Evaluates `cargo_check_v2` while retaining Cargo's final `Finished` status line.
#[must_use]
pub fn cargo_check_v2_shadow(stdout: &[u8], stderr: &[u8]) -> CargoOutputCandidate {
    cargo_check_candidate(
        stdout,
        stderr,
        CargoCheckPolicy {
            active_recovery_command: None,
            preserve_finished: true,
        },
    )
}

/// Evaluates active `cargo_check_v2` with its exact recovery command overhead.
#[must_use]
pub fn cargo_check_v2_active(
    stdout: &[u8],
    stderr: &[u8],
    recovery_command: &str,
) -> CargoOutputCandidate {
    cargo_check_candidate(
        stdout,
        stderr,
        CargoCheckPolicy {
            active_recovery_command: Some(recovery_command),
            preserve_finished: true,
        },
    )
}

/// Evaluates conservative Clippy progress stripping while preserving final status and diagnostics.
#[must_use]
pub fn cargo_clippy_v1_shadow(stdout: &[u8], stderr: &[u8]) -> CargoOutputCandidate {
    cargo_check_candidate(
        stdout,
        stderr,
        CargoCheckPolicy {
            active_recovery_command: None,
            preserve_finished: true,
        },
    )
}

fn rg_match_parts(line: &str) -> Option<(&str, &str, &str)> {
    let bytes = line.as_bytes();
    let mut found = None;
    for first in bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b':').then_some(index))
    {
        let tail = bytes.get(first.saturating_add(1)..)?;
        let Some(second_offset) = tail.iter().position(|byte| *byte == b':') else {
            continue;
        };
        let second = first.saturating_add(1).saturating_add(second_offset);
        let number = line.get(first.saturating_add(1)..second)?;
        if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
            if found.is_some() {
                return None;
            }
            found = Some((
                line.get(..first)?,
                number,
                line.get(second.saturating_add(1)..)?,
            ));
        }
    }
    found
}

/// Groups lossless `rg -n` matches under file headers without capping or truncating content.
#[must_use]
pub fn rg_v1_shadow(stdout: &[u8], stderr: &[u8]) -> SourceOutputCandidate {
    let started = Instant::now();
    let raw_bytes = u64::try_from(stdout.len().saturating_add(stderr.len())).unwrap_or(u64::MAX);
    let parsed = std::str::from_utf8(stdout).ok().and_then(|stdout| {
        let mut candidate = Vec::new();
        let mut current_file: Option<&str> = None;
        let mut grouped_match_lines = 0_u64;
        for raw_line in stdout.split_inclusive('\n') {
            let (line, ending) = raw_line
                .strip_suffix('\n')
                .map_or((raw_line, ""), |line| (line, "\n"));
            let (path, number, content) = rg_match_parts(line)?;
            if current_file != Some(path) {
                candidate.extend_from_slice(path.as_bytes());
                candidate.extend_from_slice(b":\n");
                current_file = Some(path);
            }
            candidate.extend_from_slice(b"  ");
            candidate.extend_from_slice(number.as_bytes());
            candidate.push(b':');
            candidate.extend_from_slice(content.as_bytes());
            candidate.extend_from_slice(ending.as_bytes());
            grouped_match_lines = grouped_match_lines.saturating_add(1);
        }
        (grouped_match_lines > 0).then_some((candidate, grouped_match_lines))
    });
    let (candidate_stdout, grouped_match_lines, applicable) = parsed.map_or_else(
        || (stdout.to_vec(), 0, false),
        |(candidate, lines)| (candidate, lines, true),
    );
    let candidate_stderr = stderr.to_vec();
    let candidate_bytes = u64::try_from(
        candidate_stdout
            .len()
            .saturating_add(candidate_stderr.len()),
    )
    .unwrap_or(u64::MAX);
    let estimated_raw_tokens = estimated_tokens(raw_bytes);
    let estimated_candidate_tokens = estimated_tokens(candidate_bytes);
    let never_worse_accepted = applicable
        && candidate_bytes < raw_bytes
        && estimated_candidate_tokens < estimated_raw_tokens;
    SourceOutputCandidate {
        candidate_stdout,
        candidate_stderr,
        raw_bytes,
        candidate_bytes,
        estimated_raw_tokens,
        estimated_candidate_tokens,
        omitted_passing_tests: 0,
        omitted_progress_lines: 0,
        grouped_match_lines,
        recovery_hint_bytes: 0,
        reducer_duration: started.elapsed(),
        applicable,
        never_worse_accepted,
    }
}

/// Executes a previously admitted simple command without filtering its bytes.
///
/// # Errors
///
/// Returns an error when the command is not admitted or the real process cannot be executed.
pub fn execute_passthrough(
    command: &str,
    ids: &UuidV7Generator,
) -> Result<(Vec<u8>, Vec<u8>, SourceExecutionMetadata), std::io::Error> {
    let RewriteDecision::Passthrough { family, contract } = decide(command) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "command is not admitted",
        ));
    };
    let mut words = command.split_ascii_whitespace();
    let Some(program) = words.next() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        ));
    };
    let started = Instant::now();
    let executable = if program == "cargo" {
        std::env::var_os("TRACEPRESS_REAL_CARGO").unwrap_or_else(|| program.into())
    } else {
        program.into()
    };
    let output = Command::new(executable).args(words).output()?;
    let stdout = output.stdout;
    let stderr = output.stderr;
    let raw_stdout_bytes = u64::try_from(stdout.len()).unwrap_or(u64::MAX);
    let raw_stderr_bytes = u64::try_from(stderr.len()).unwrap_or(u64::MAX);
    let metadata = SourceExecutionMetadata {
        source_execution_id: SourceExecutionId::generate(ids),
        command_family: family,
        output_contract: contract,
        raw_stdout_bytes,
        raw_stderr_bytes,
        emitted_bytes: raw_stdout_bytes.saturating_add(raw_stderr_bytes),
        exit_code: output.status.code(),
        #[cfg(unix)]
        termination_signal: output.status.signal(),
        #[cfg(not(unix))]
        termination_signal: None,
        duration: started.elapsed(),
    };
    Ok((stdout, stderr, metadata))
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "test assertion setup uses explicit panic branches instead of banned unwrap/expect"
)]
mod tests {
    use super::*;
    #[test]
    fn admits_only_supported_standalone_cargo_commands() {
        assert!(matches!(
            decide("cargo test -q"),
            RewriteDecision::Passthrough {
                family: CommandFamily::CargoTest,
                ..
            }
        ));
        assert!(matches!(
            decide("cargo check --workspace"),
            RewriteDecision::Passthrough {
                family: CommandFamily::CargoCheck,
                ..
            }
        ));
        assert!(matches!(
            decide("cargo clippy --workspace"),
            RewriteDecision::Passthrough {
                family: CommandFamily::CargoClippy,
                ..
            }
        ));
        assert!(matches!(
            decide("rg -n fn crates"),
            RewriteDecision::Passthrough {
                family: CommandFamily::Ripgrep,
                ..
            }
        ));
        assert_eq!(
            decide("cargo build"),
            RewriteDecision::FailOpen(FailOpenReason::Unsupported)
        );
    }
    #[test]
    fn shell_composition_fails_open() {
        for value in [
            "cargo test | tail",
            "cargo test > log",
            "cargo test && git status",
            "$(cargo test)",
            "cargo test 'quoted test'",
            "cargo test \\\n+              --all",
        ] {
            assert_eq!(
                decide(value),
                RewriteDecision::FailOpen(FailOpenReason::ShellSyntax)
            );
        }
    }

    #[test]
    fn codex_rewrite_is_narrow_and_does_not_grant_permission() {
        let input = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test -q"}}"#;
        let Some(output) = codex_pre_tool_use_rewrite(input) else {
            panic!("admitted command must rewrite");
        };
        let Ok(value): Result<serde_json::Value, _> = serde_json::from_slice(&output) else {
            panic!("rewrite response must be valid JSON");
        };
        assert!(
            value
                .pointer("/hookSpecificOutput/updatedInput/command")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|command| command.ends_with(" tool cargo test -q"))
        );
        assert_eq!(
            value
                .pointer("/hookSpecificOutput/permissionDecision")
                .and_then(serde_json::Value::as_str),
            Some("allow")
        );
        assert!(codex_pre_tool_use_rewrite(br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test | tail"}}"#).is_none());
    }

    #[test]
    fn identity_rewrite_preserves_the_admitted_command() {
        let input = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test -q"}}"#;
        let Some(output) = codex_pre_tool_use_identity_rewrite(input) else {
            panic!("admitted command must produce an identity rewrite");
        };
        let Ok(value): Result<serde_json::Value, _> = serde_json::from_slice(&output) else {
            panic!("identity response must be valid JSON");
        };
        assert_eq!(
            value
                .pointer("/hookSpecificOutput/updatedInput/command")
                .and_then(serde_json::Value::as_str),
            Some("cargo test -q")
        );
    }

    #[test]
    fn cargo_test_shadow_preserves_failures_and_drops_passing_noise() {
        let stdout = b"running 2 tests\ntest a ... ok\ntest b ... FAILED\n\nfailures:\nboom\ntest result: FAILED. 1 passed; 1 failed\n";
        let stderr = b"   Compiling demo v0.1.0\nerror: compilation detail\n";
        let candidate = cargo_test_v1_shadow(stdout, stderr);
        assert!(candidate.applicable);
        let combined = [candidate.candidate_stdout, candidate.candidate_stderr].concat();
        let Ok(text) = String::from_utf8(combined) else {
            panic!("candidate must remain UTF-8");
        };
        assert!(text.contains("test b ... FAILED"));
        assert!(text.contains("boom"));
        assert!(text.contains("error: compilation detail"));
        assert!(!text.contains("test a ... ok"));
    }

    #[test]
    fn cargo_test_shadow_rejects_non_improving_output() {
        let candidate = cargo_test_v1_shadow(b"test result: ok. 0 passed\n", b"");
        assert!(!candidate.applicable);
        assert!(!candidate.never_worse_accepted);
    }

    #[test]
    fn cargo_test_shadow_accepts_material_reduction() {
        use std::fmt::Write as _;
        let mut stdout = String::from("running 50 tests\n");
        for index in 0..50 {
            let _written = writeln!(&mut stdout, "test suite::case_{index} ... ok");
        }
        stdout.push_str("test result: ok. 50 passed; 0 failed\n");
        let candidate = cargo_test_v1_shadow(stdout.as_bytes(), b"    Finished test profile\n");
        assert!(candidate.never_worse_accepted);
        assert!(candidate.candidate_bytes < candidate.raw_bytes);
        assert!(candidate.estimated_candidate_tokens < candidate.estimated_raw_tokens);
        assert!(candidate.recovery_hint_bytes > 0);
    }

    #[test]
    fn cargo_test_active_counts_exact_recovery_hint_in_never_worse() {
        let stdout = b"running 3 tests\ntest a ... ok\ntest b ... ok\ntest c ... ok\ntest result: ok. 3 passed; 0 failed\n";
        let candidate = cargo_test_v1_active(stdout, b"", "tracepress recall deadbeef");
        let text = String::from_utf8_lossy(&candidate.candidate_stdout);
        assert!(text.contains("tracepress recall deadbeef"));
        assert_eq!(
            candidate.recovery_hint_bytes,
            u64::try_from(text.lines().next().map_or(0, |line| line.len() + 1)).unwrap_or(u64::MAX)
        );
        assert_eq!(
            candidate.candidate_bytes,
            u64::try_from(text.len()).unwrap_or(u64::MAX)
        );
    }

    #[test]
    fn cargo_check_shadow_drops_only_progress_and_preserves_diagnostics() {
        let stdout = b"build script note\n";
        let mut stderr = String::new();
        for index in 0..12 {
            use std::fmt::Write as _;
            let _written = writeln!(&mut stderr, "   Checking dependency-{index} v0.1.0");
        }
        stderr.push_str(
            "warning: unused item\n --> src/lib.rs:1:1\nerror: stop\n   Finished dev profile\n",
        );
        let candidate = cargo_check_v1_shadow(stdout, stderr.as_bytes());
        assert!(candidate.applicable);
        assert!(candidate.never_worse_accepted);
        assert_eq!(candidate.omitted_progress_lines, 13);
        assert_eq!(candidate.omitted_passing_tests, 0);
        let combined = [candidate.candidate_stdout, candidate.candidate_stderr].concat();
        let text = String::from_utf8_lossy(&combined);
        assert!(text.contains("build script note"));
        assert!(text.contains("warning: unused item"));
        assert!(text.contains("src/lib.rs:1:1"));
        assert!(text.contains("error: stop"));
        assert!(!text.contains("Checking dependency"));
        assert!(!text.contains("Finished dev"));
    }

    #[test]
    fn cargo_check_shadow_fails_closed_on_non_utf8_stderr() {
        let candidate = cargo_check_v1_shadow(b"", &[0xff, b'\n']);
        assert!(!candidate.applicable);
        assert!(!candidate.never_worse_accepted);
        assert_eq!(candidate.candidate_stderr, vec![0xff, b'\n']);
    }

    #[test]
    fn cargo_check_active_counts_exact_recovery_hint_in_never_worse() {
        let mut stderr = String::new();
        for index in 0..12 {
            use std::fmt::Write as _;
            let _written = writeln!(&mut stderr, "   Checking dependency-{index} v0.1.0");
        }
        let candidate = cargo_check_v1_active(b"", stderr.as_bytes(), "tracepress recall deadbeef");
        let text = String::from_utf8_lossy(&candidate.candidate_stdout);
        assert!(candidate.never_worse_accepted);
        assert!(text.contains("tracepress recall deadbeef"));
        assert_eq!(
            candidate.recovery_hint_bytes,
            u64::try_from(text.len()).unwrap_or(u64::MAX)
        );
    }

    #[test]
    fn cargo_check_v2_preserves_final_status_while_dropping_progress() {
        let mut stderr = String::new();
        for index in 0..12 {
            use std::fmt::Write as _;
            let _written = writeln!(&mut stderr, "   Checking dependency-{index} v0.1.0");
        }
        stderr.push_str("    Finished dev profile in 1.0s\n");
        let candidate = cargo_check_v2_shadow(b"", stderr.as_bytes());
        let text = String::from_utf8_lossy(&candidate.candidate_stderr);
        assert!(candidate.never_worse_accepted);
        assert_eq!(candidate.omitted_progress_lines, 12);
        assert!(text.contains("Finished dev profile"));
        assert!(!text.contains("Checking dependency"));
    }

    #[test]
    fn cargo_clippy_shadow_preserves_lints_locations_and_final_status() {
        let mut stderr = String::new();
        for index in 0..12 {
            use std::fmt::Write as _;
            let _written = writeln!(&mut stderr, "    Checking dependency-{index} v0.1.0");
        }
        stderr.push_str(
            "warning: redundant closure\n --> src/lib.rs:2:3\n    Finished dev profile in 1.0s\n",
        );
        let candidate = cargo_clippy_v1_shadow(b"", stderr.as_bytes());
        let text = String::from_utf8_lossy(&candidate.candidate_stderr);
        assert!(candidate.never_worse_accepted);
        assert_eq!(candidate.omitted_progress_lines, 12);
        assert!(text.contains("warning: redundant closure"));
        assert!(text.contains("src/lib.rs:2:3"));
        assert!(text.contains("Finished dev profile"));
        assert!(!text.contains("Checking dependency"));
    }

    #[test]
    fn rg_shadow_groups_matches_without_losing_content() {
        use std::fmt::Write as _;
        let mut stdout = String::new();
        for index in 1..=12 {
            let _written = writeln!(
                &mut stdout,
                "crates/tracepress-example/src/lib.rs:{index}:fn case_{index}() {{}}"
            );
        }
        let candidate = rg_v1_shadow(stdout.as_bytes(), b"diagnostic\n");
        assert!(candidate.applicable);
        assert!(candidate.never_worse_accepted);
        assert_eq!(candidate.grouped_match_lines, 12);
        assert_eq!(candidate.candidate_stderr, b"diagnostic\n");
        let text = String::from_utf8_lossy(&candidate.candidate_stdout);
        assert_eq!(
            text.matches("crates/tracepress-example/src/lib.rs:\n")
                .count(),
            1
        );
        for index in 1..=12 {
            assert!(text.contains(&format!("  {index}:fn case_{index}() {{}}")));
        }
    }

    #[test]
    fn rg_shadow_fails_closed_on_ambiguous_machine_shape() {
        let stdout = b"path:12:value:34:other\n";
        let candidate = rg_v1_shadow(stdout, b"");
        assert!(!candidate.applicable);
        assert!(!candidate.never_worse_accepted);
        assert_eq!(candidate.candidate_stdout, stdout);
    }
}
