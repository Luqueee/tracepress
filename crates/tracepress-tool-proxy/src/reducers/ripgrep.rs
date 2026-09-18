use std::time::Instant;

use super::estimated_tokens;
use crate::model::SourceOutputCandidate;

fn match_parts(line: &str) -> Option<(&str, &str, &str)> {
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

fn candidate(
    stdout: &[u8],
    stderr: &[u8],
    active_recovery_command: Option<&str>,
) -> SourceOutputCandidate {
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
            let (path, number, content) = match_parts(line)?;
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
    let (mut candidate_stdout, grouped_match_lines, applicable) = parsed.map_or_else(
        || (stdout.to_vec(), 0, false),
        |(candidate, lines)| (candidate, lines, true),
    );
    let recovery_hint_bytes = if applicable {
        active_recovery_command.map_or(0, |command| {
            let hint = format!(
                "[Tracepress: grouped {grouped_match_lines} matches by file. Exact output: {command}]\n"
            );
            let hint_bytes = u64::try_from(hint.len()).unwrap_or(u64::MAX);
            let mut prefixed = hint.into_bytes();
            prefixed.append(&mut candidate_stdout);
            candidate_stdout = prefixed;
            hint_bytes
        })
    } else {
        0
    };
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
        omitted_advisory_lines: 0,
        grouped_match_lines,
        recovery_hint_bytes,
        reducer_duration: started.elapsed(),
        applicable,
        never_worse_accepted,
    }
}

#[must_use]
pub fn rg_v1_shadow(stdout: &[u8], stderr: &[u8]) -> SourceOutputCandidate {
    candidate(stdout, stderr, None)
}

#[must_use]
pub fn rg_v1_active(stdout: &[u8], stderr: &[u8], recovery_command: &str) -> SourceOutputCandidate {
    candidate(stdout, stderr, Some(recovery_command))
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn output() -> String {
        let mut stdout = String::new();
        for index in 1..=12 {
            let _written = writeln!(
                &mut stdout,
                "crates/tracepress-example/src/lib.rs:{index}:fn case_{index}() {{}}"
            );
        }
        stdout
    }

    #[test]
    fn shadow_groups_matches_without_losing_content() {
        let result = rg_v1_shadow(output().as_bytes(), b"diagnostic\n");
        assert!(result.applicable);
        assert!(result.never_worse_accepted);
        assert_eq!(result.grouped_match_lines, 12);
        assert_eq!(result.candidate_stderr, b"diagnostic\n");
        let text = String::from_utf8_lossy(&result.candidate_stdout);
        assert_eq!(
            text.matches("crates/tracepress-example/src/lib.rs:\n")
                .count(),
            1
        );
    }

    #[test]
    fn shadow_fails_closed_on_ambiguous_machine_shape() {
        let stdout = b"path:12:value:34:other\n";
        let result = rg_v1_shadow(stdout, b"");
        assert!(!result.applicable);
        assert!(!result.never_worse_accepted);
        assert_eq!(result.candidate_stdout, stdout);
    }

    #[test]
    fn active_counts_exact_recovery_hint_in_never_worse() {
        let result = rg_v1_active(output().as_bytes(), b"", "tracepress recall deadbeef");
        let text = String::from_utf8_lossy(&result.candidate_stdout);
        assert!(result.never_worse_accepted);
        assert!(text.contains("tracepress recall deadbeef"));
        assert_eq!(
            result.recovery_hint_bytes,
            u64::try_from(text.lines().next().map_or(0, |line| line.len() + 1)).unwrap_or(u64::MAX)
        );
    }

    #[test]
    fn active_ambiguous_output_fails_open_without_recovery_hint() {
        let stdout = b"path:12:value:34:other\n";
        let result = rg_v1_active(stdout, b"diagnostic\n", "tracepress recall deadbeef");
        assert!(!result.applicable);
        assert!(!result.never_worse_accepted);
        assert_eq!(result.recovery_hint_bytes, 0);
        assert_eq!(result.candidate_stdout, stdout);
        assert_eq!(result.candidate_stderr, b"diagnostic\n");
    }
}
