use std::time::Instant;

use super::estimated_tokens;
use crate::model::CargoOutputCandidate;

#[derive(Clone, Copy)]
struct Policy<'a> {
    active_recovery_command: Option<&'a str>,
    preserve_finished: bool,
}

fn candidate(stdout: &[u8], stderr: &[u8], policy: Policy<'_>) -> CargoOutputCandidate {
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
        omitted_advisory_lines: 0,
        grouped_match_lines: 0,
        recovery_hint_bytes,
        reducer_duration: started.elapsed(),
        applicable,
        never_worse_accepted,
    }
}

#[must_use]
pub fn cargo_check_v1_shadow(stdout: &[u8], stderr: &[u8]) -> CargoOutputCandidate {
    candidate(
        stdout,
        stderr,
        Policy {
            active_recovery_command: None,
            preserve_finished: false,
        },
    )
}

#[must_use]
pub fn cargo_check_v1_active(
    stdout: &[u8],
    stderr: &[u8],
    recovery_command: &str,
) -> CargoOutputCandidate {
    candidate(
        stdout,
        stderr,
        Policy {
            active_recovery_command: Some(recovery_command),
            preserve_finished: false,
        },
    )
}

#[must_use]
pub fn cargo_check_v2_shadow(stdout: &[u8], stderr: &[u8]) -> CargoOutputCandidate {
    candidate(
        stdout,
        stderr,
        Policy {
            active_recovery_command: None,
            preserve_finished: true,
        },
    )
}

#[must_use]
pub fn cargo_check_v2_active(
    stdout: &[u8],
    stderr: &[u8],
    recovery_command: &str,
) -> CargoOutputCandidate {
    candidate(
        stdout,
        stderr,
        Policy {
            active_recovery_command: Some(recovery_command),
            preserve_finished: true,
        },
    )
}

#[must_use]
pub fn cargo_clippy_v1_shadow(stdout: &[u8], stderr: &[u8]) -> CargoOutputCandidate {
    cargo_check_v2_shadow(stdout, stderr)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn progress() -> String {
        let mut stderr = String::new();
        for index in 0..12 {
            let _written = writeln!(&mut stderr, "   Checking dependency-{index} v0.1.0");
        }
        stderr
    }

    #[test]
    fn check_v1_drops_progress_and_preserves_diagnostics() {
        let mut stderr = progress();
        stderr.push_str(
            "warning: unused item\n --> src/lib.rs:1:1\nerror: stop\n   Finished dev profile\n",
        );
        let result = cargo_check_v1_shadow(b"build script note\n", stderr.as_bytes());
        assert!(result.applicable);
        assert!(result.never_worse_accepted);
        assert_eq!(result.omitted_progress_lines, 13);
        let text = String::from_utf8_lossy(&result.candidate_stderr);
        assert!(text.contains("warning: unused item"));
        assert!(text.contains("src/lib.rs:1:1"));
        assert!(text.contains("error: stop"));
        assert!(!text.contains("Finished dev"));
    }

    #[test]
    fn check_fails_closed_on_non_utf8_stderr() {
        let result = cargo_check_v1_shadow(b"", &[0xff, b'\n']);
        assert!(!result.applicable);
        assert!(!result.never_worse_accepted);
        assert_eq!(result.candidate_stderr, vec![0xff, b'\n']);
    }

    #[test]
    fn check_v1_active_counts_exact_recovery_hint() {
        let result =
            cargo_check_v1_active(b"", progress().as_bytes(), "tracepress recall deadbeef");
        let text = String::from_utf8_lossy(&result.candidate_stdout);
        assert!(result.never_worse_accepted);
        assert!(text.contains("tracepress recall deadbeef"));
        assert_eq!(
            result.recovery_hint_bytes,
            u64::try_from(text.len()).unwrap_or(u64::MAX)
        );
    }

    #[test]
    fn check_v2_preserves_final_status() {
        let mut stderr = progress();
        stderr.push_str("    Finished dev profile in 1.0s\n");
        let result = cargo_check_v2_shadow(b"", stderr.as_bytes());
        let text = String::from_utf8_lossy(&result.candidate_stderr);
        assert!(result.never_worse_accepted);
        assert_eq!(result.omitted_progress_lines, 12);
        assert!(text.contains("Finished dev profile"));
    }

    #[test]
    fn clippy_preserves_lints_locations_and_final_status() {
        let mut stderr = progress();
        stderr.push_str(
            "warning: redundant closure\n --> src/lib.rs:2:3\n    Finished dev profile in 1.0s\n",
        );
        let result = cargo_clippy_v1_shadow(b"", stderr.as_bytes());
        let text = String::from_utf8_lossy(&result.candidate_stderr);
        assert!(result.never_worse_accepted);
        assert!(text.contains("warning: redundant closure"));
        assert!(text.contains("src/lib.rs:2:3"));
        assert!(text.contains("Finished dev profile"));
    }
}
