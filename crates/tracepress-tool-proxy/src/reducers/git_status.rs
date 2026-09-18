use std::time::Instant;

use super::estimated_tokens;
use crate::model::SourceOutputCandidate;

#[must_use]
pub fn git_status_v1_shadow(stdout: &[u8], stderr: &[u8]) -> SourceOutputCandidate {
    let started = Instant::now();
    let raw_bytes = u64::try_from(stdout.len().saturating_add(stderr.len())).unwrap_or(u64::MAX);
    let parsed = std::str::from_utf8(stdout).ok().and_then(|stdout| {
        let recognized_header = stdout.lines().next().is_some_and(|line| {
            line.starts_with("On branch ")
                || line.starts_with("HEAD detached at ")
                || line.starts_with("HEAD detached from ")
                || line == "Not currently on any branch."
        });
        let recognized_section = [
            "Changes to be committed:\n",
            "Changes not staged for commit:\n",
            "Untracked files:\n",
            "Unmerged paths:\n",
        ]
        .iter()
        .any(|heading| stdout.contains(heading));
        if !recognized_header || !recognized_section {
            return None;
        }
        let mut candidate = Vec::new();
        let mut omitted_advisory_lines = 0_u64;
        for line in stdout.split_inclusive('\n') {
            if line.starts_with("  (use \"") && line.trim_end().ends_with(')') {
                omitted_advisory_lines = omitted_advisory_lines.saturating_add(1);
            } else {
                candidate.extend_from_slice(line.as_bytes());
            }
        }
        (omitted_advisory_lines > 0).then_some((candidate, omitted_advisory_lines))
    });
    let (candidate_stdout, omitted_advisory_lines, applicable) = parsed.map_or_else(
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
        omitted_advisory_lines,
        grouped_match_lines: 0,
        recovery_hint_bytes: 0,
        reducer_duration: started.elapsed(),
        applicable,
        never_worse_accepted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_advisory_lines() {
        let stdout = b"On branch main\n\nChanges to be committed:\n  (use \"git restore --staged <file>...\" to unstage)\n\n\tnew file:   staged.txt\n\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n  (use \"git restore <file>...\" to discard changes in working directory)\n\n\tmodified:   tracked.txt\n\nUntracked files:\n  (use \"git add <file>...\" to include in what will be committed)\n\n\tuntracked.txt\n\nno changes added to commit (use \"git add\" and/or \"git commit -a\")\n";
        let expected = b"On branch main\n\nChanges to be committed:\n\n\tnew file:   staged.txt\n\nChanges not staged for commit:\n\n\tmodified:   tracked.txt\n\nUntracked files:\n\n\tuntracked.txt\n\nno changes added to commit (use \"git add\" and/or \"git commit -a\")\n";
        let result = git_status_v1_shadow(stdout, b"diagnostic\n");
        assert!(result.applicable);
        assert!(result.never_worse_accepted);
        assert_eq!(result.omitted_advisory_lines, 4);
        assert_eq!(result.candidate_stdout, expected);
        assert_eq!(result.candidate_stderr, b"diagnostic\n");
    }

    #[test]
    fn supports_detached_head_but_not_clean_or_unknown_output() {
        let detached = b"HEAD detached at 3fce3b5b\nUntracked files:\n  (use \"git add <file>...\" to include in what will be committed)\n\n\tscratch.txt\n\nnothing added to commit but untracked files present (use \"git add\" to track)\n";
        let result = git_status_v1_shadow(detached, b"");
        assert!(result.applicable);
        assert!(result.never_worse_accepted);
        assert_eq!(result.omitted_advisory_lines, 1);
        for raw in [
            b"On branch main\nnothing to commit, working tree clean\n".as_slice(),
            b"etat de la copie de travail inconnu\n".as_slice(),
            &[0xff, b'\n'],
        ] {
            let result = git_status_v1_shadow(raw, b"");
            assert!(!result.applicable);
            assert!(!result.never_worse_accepted);
            assert_eq!(result.candidate_stdout, raw);
        }
    }
}
