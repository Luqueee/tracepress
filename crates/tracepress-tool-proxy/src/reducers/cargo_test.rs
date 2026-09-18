use std::time::Instant;

use super::estimated_tokens;
use crate::model::CargoOutputCandidate;

fn streams(stdout: &str, stderr: &str) -> (Vec<u8>, Vec<u8>, u64, u64) {
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

fn candidate(
    stdout: &[u8],
    stderr: &[u8],
    active_recovery_command: Option<&str>,
) -> CargoOutputCandidate {
    let started = Instant::now();
    let raw_bytes = u64::try_from(stdout.len().saturating_add(stderr.len())).unwrap_or(u64::MAX);
    let (mut candidate_stdout, candidate_stderr, omitted_passing_tests, omitted_progress_lines) =
        match (std::str::from_utf8(stdout), std::str::from_utf8(stderr)) {
            (Ok(stdout), Ok(stderr)) => streams(stdout, stderr),
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
        omitted_advisory_lines: 0,
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
    candidate(stdout, stderr, None)
}

/// Evaluates `cargo_test_v1` with the exact recovery command included in never-worse.
#[must_use]
pub fn cargo_test_v1_active(
    stdout: &[u8],
    stderr: &[u8],
    recovery_command: &str,
) -> CargoOutputCandidate {
    candidate(stdout, stderr, Some(recovery_command))
}

#[cfg(test)]
#[allow(clippy::panic, reason = "test fixtures use explicit failure branches")]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn shadow_preserves_failures_and_drops_passing_noise() {
        let stdout = b"running 2 tests\ntest a ... ok\ntest b ... FAILED\n\nfailures:\nboom\ntest result: FAILED. 1 passed; 1 failed\n";
        let stderr = b"   Compiling demo v0.1.0\nerror: compilation detail\n";
        let result = cargo_test_v1_shadow(stdout, stderr);
        assert!(result.applicable);
        let Ok(text) =
            String::from_utf8([result.candidate_stdout, result.candidate_stderr].concat())
        else {
            panic!("candidate must remain UTF-8");
        };
        assert!(text.contains("test b ... FAILED"));
        assert!(text.contains("boom"));
        assert!(text.contains("error: compilation detail"));
        assert!(!text.contains("test a ... ok"));
    }

    #[test]
    fn shadow_rejects_non_improving_output() {
        let result = cargo_test_v1_shadow(b"test result: ok. 0 passed\n", b"");
        assert!(!result.applicable);
        assert!(!result.never_worse_accepted);
    }

    #[test]
    fn shadow_accepts_material_reduction() {
        let mut stdout = String::from("running 50 tests\n");
        for index in 0..50 {
            let _written = writeln!(&mut stdout, "test suite::case_{index} ... ok");
        }
        stdout.push_str("test result: ok. 50 passed; 0 failed\n");
        let result = cargo_test_v1_shadow(stdout.as_bytes(), b"    Finished test profile\n");
        assert!(result.never_worse_accepted);
        assert!(result.candidate_bytes < result.raw_bytes);
        assert!(result.estimated_candidate_tokens < result.estimated_raw_tokens);
        assert!(result.recovery_hint_bytes > 0);
    }

    #[test]
    fn active_counts_exact_recovery_hint_in_never_worse() {
        let stdout = b"running 3 tests\ntest a ... ok\ntest b ... ok\ntest c ... ok\ntest result: ok. 3 passed; 0 failed\n";
        let result = cargo_test_v1_active(stdout, b"", "tracepress recall deadbeef");
        let text = String::from_utf8_lossy(&result.candidate_stdout);
        assert!(text.contains("tracepress recall deadbeef"));
        assert_eq!(
            result.recovery_hint_bytes,
            u64::try_from(text.lines().next().map_or(0, |line| line.len() + 1)).unwrap_or(u64::MAX)
        );
        assert_eq!(
            result.candidate_bytes,
            u64::try_from(text.len()).unwrap_or(u64::MAX)
        );
    }
}
