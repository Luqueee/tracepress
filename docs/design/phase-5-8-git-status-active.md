# Phase 5.8: `git_status_v1` active pilot

## Scope

Phase 5.8 evaluates the Phase 5.7 advisory-only projection under explicit
`TRACEPRESS_SOURCE_REDUCER=git_status_v1_active`. It changes no default policy and admits no new
command syntax. Only exact standalone `git status` remains eligible.

## Active contract

Recognized dirty status output removes only Git's parenthesized advice lines and prepends a bounded
hint containing the exact opaque recovery command. Hint bytes participate in byte and estimated-token
never-worse checks. Accepted candidates create session-scoped byte-faithful recovery state before
being emitted.

Clean, localized, non-UTF-8, unrecognized, or non-improving output emits raw stdout/stderr, records
fail-open, and creates no recovery object. Exit codes and stderr remain unchanged.

## Experiment design

Two independent N=10 paired cohorts use the pinned public ripgrep repository, alternating order,
isolated state, and session-level assignment:

- dirty: identical staged, modified, and untracked fixtures in every arm;
- clean: untouched detached worktrees exercising raw fail-open.

The first dirty smoke used a verbose 161-byte recovery hint and achieved only 19.65% reduction,
below the unchanged 20% gate. A shorter but complete hint reduced overhead to 142 bytes and passed a
second smoke at 23.84%. Both smoke artifacts are retained.

The full dirty gate requires material source reduction, lower aggregate uncached input, negative
paired median uncached delta, objective task success, no material request/tool/retry increase, and
low recovery. The clean safety gate requires exact raw emission, zero mutations and recoveries, and
successful tasks.
