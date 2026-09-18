# Phase 5.7: `git_status_v1` Shadow

## Scope

Phase 5.7 admits only the exact standalone command `git status`. Flags, pipelines, redirections,
substitutions, compound syntax, quoting, globs, and unknown syntax remain fail-open. The reducer is
available only through `TRACEPRESS_SOURCE_REDUCER=git_status_v1_shadow`, and Shadow always returns
the original stdout and stderr to the agent.

## Candidate contract

V1 recognizes UTF-8 long-form Git status output beginning with a branch or detached-HEAD header and
containing at least one known change section. It removes only Git's two-space-indented,
parenthesized `use` guidance lines. It preserves branch state, tracking information, section
headings, staged/modified/deleted/renamed/unmerged/untracked entries, paths, blank-line structure,
the final status summary, stderr, and process status.

Clean output has no removable guidance and therefore returns a raw-equivalent, non-applicable
candidate. Localized, non-UTF-8, headerless, or sectionless output also fails closed to raw. No
path grouping, truncation, deduplication, status normalization, or recovery infrastructure is
introduced.

## Measured workload and gate

The public pinned ripgrep worktree receives the same deterministic staged, modified, and untracked
fixture in each isolated arm. Control and Shadow execute the exact same wrapper command in
alternating order. Reports retain aggregate measurements only and do not persist fixture names or
status output.

Passing requires at least 20% candidate reduction, raw byte-identical forwarding, one accepted
Shadow evaluation per Treatment session, objective task success, zero retry/recovery regression,
and reducer latency below 50 ms. Provider deltas cannot be attributed to the candidate while it is
not emitted.
