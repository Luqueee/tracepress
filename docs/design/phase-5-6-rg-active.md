# Phase 5.6: `rg_v1` active pilot

## Scope

Phase 5.6 activates the lossless file-grouping candidate accepted in Phase 5.5, only through the
explicit `TRACEPRESS_SOURCE_REDUCER=rg_v1_active` opt-in. It does not alter default runtime policy,
admit new command families, cap results, truncate lines, or deduplicate matches.

The existing shell-consumer boundary remains authoritative: only standalone simple `rg` commands
are admitted. Pipelines, redirections, substitutions, compound syntax, quoting, globs, and unknown
syntax fail open before execution.

## Active contract

The reducer accepts UTF-8 stdout only when every line has exactly one unambiguous
`path:line:content` boundary. It groups consecutive matches under their file header while retaining
every path, line number, content byte, line ending, and stderr byte. The active candidate prepends a
bounded recovery hint containing an opaque, session-scoped recovery command. Hint bytes participate
in both byte and estimated-token never-worse checks.

If parsing is ambiguous or never-worse fails, Tracepress emits raw stdout and stderr, preserves the
process exit contract, records a fail-open reason, and does not create a recovery object. Accepted
candidates store byte-faithful raw output temporarily; recovery remains explicit and was not needed
in the measured cohort.

## Experiment design

Two independent N=10 paired cohorts use the pinned public ripgrep repository, alternating arm order,
clean detached worktrees, isolated state, and session-level assignment:

- parseable: the same 415-line workload selected in Phase 5.5;
- ambiguous: the 2,981-line workload whose four ambiguous lines force whole-stream fail-open.

The parseable gate requires material source reduction, lower aggregate uncached input, a negative
paired median uncached delta, objective task success, no material request/tool/retry increase, and a
low recovery rate. The ambiguous gate requires exact raw emission, zero forwarding mutations, zero
recoveries, successful tasks, and bounded trajectory noise.
