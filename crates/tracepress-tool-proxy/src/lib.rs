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
    let output = Command::new(program).args(words).output()?;
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
}
