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
pub enum OutputContract {
    AgentReadable,
    LineOrientedMachine,
    StructuredMachine,
    Unknown,
}

/// A supported command family. Phase 5.0 deliberately starts with one family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandFamily {
    CargoTest,
}

/// Why a command was left untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailOpenReason {
    Unsupported,
    ShellSyntax,
}

/// The only admission outcome available before reducers exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    if matches!(words.as_slice(), ["cargo", "test", ..]) {
        RewriteDecision::Passthrough {
            family: CommandFamily::CargoTest,
            contract: OutputContract::AgentReadable,
        }
    } else {
        RewriteDecision::FailOpen(FailOpenReason::Unsupported)
    }
}

/// Allowlisted metadata for one source execution; it intentionally has no command or output.
#[derive(Clone, Debug, Eq, PartialEq)]
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
pub struct CargoTestShadowCandidate {
    pub candidate_stdout: Vec<u8>,
    pub candidate_stderr: Vec<u8>,
    pub raw_bytes: u64,
    pub candidate_bytes: u64,
    pub estimated_raw_tokens: u64,
    pub estimated_candidate_tokens: u64,
    pub omitted_passing_tests: u64,
    pub omitted_progress_lines: u64,
    pub applicable: bool,
    pub never_worse_accepted: bool,
}

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
    if omitted_passing_tests > 0 {
        let hint = format!(
            "[Tracepress: {omitted_passing_tests} passing test lines omitted; full output recoverable when active.]\n"
        );
        let mut prefixed = hint.into_bytes();
        prefixed.append(&mut candidate_stdout);
        candidate_stdout = prefixed;
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

/// Evaluates `cargo_test_v1` without changing agent-visible bytes.
#[must_use]
pub fn cargo_test_v1_shadow(stdout: &[u8], stderr: &[u8]) -> CargoTestShadowCandidate {
    let raw_bytes = u64::try_from(stdout.len().saturating_add(stderr.len())).unwrap_or(u64::MAX);
    let (candidate_stdout, candidate_stderr, omitted_passing_tests, omitted_progress_lines) =
        match (std::str::from_utf8(stdout), std::str::from_utf8(stderr)) {
            (Ok(stdout), Ok(stderr)) => cargo_test_v1_streams(stdout, stderr),
            _ => (stdout.to_vec(), stderr.to_vec(), 0, 0),
        };
    let candidate_bytes = u64::try_from(
        candidate_stdout
            .len()
            .saturating_add(candidate_stderr.len()),
    )
    .unwrap_or(u64::MAX);
    let estimated_raw_tokens = estimated_tokens(raw_bytes);
    let estimated_candidate_tokens = estimated_tokens(candidate_bytes);
    let applicable = omitted_passing_tests > 0 || omitted_progress_lines > 0;
    let never_worse_accepted = applicable
        && candidate_bytes < raw_bytes
        && estimated_candidate_tokens < estimated_raw_tokens;
    CargoTestShadowCandidate {
        candidate_stdout,
        candidate_stderr,
        raw_bytes,
        candidate_bytes,
        estimated_raw_tokens,
        estimated_candidate_tokens,
        omitted_passing_tests,
        omitted_progress_lines,
        applicable,
        never_worse_accepted,
    }
}

/// Executes a previously admitted simple command without filtering its bytes.
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
mod tests {
    use super::*;
    #[test]
    fn admits_only_standalone_cargo_test() {
        assert!(matches!(
            decide("cargo test -q"),
            RewriteDecision::Passthrough {
                family: CommandFamily::CargoTest,
                ..
            }
        ));
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
        let output = codex_pre_tool_use_rewrite(input).expect("admitted command must rewrite");
        let value: serde_json::Value = serde_json::from_slice(&output).expect("valid response");
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
        let output = codex_pre_tool_use_identity_rewrite(input).expect("identity rewrite");
        let value: serde_json::Value = serde_json::from_slice(&output).expect("valid response");
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
        let text = String::from_utf8(combined).expect("candidate remains utf8");
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
    }
}
