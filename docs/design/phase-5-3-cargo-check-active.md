# Phase 5.3: `cargo_check_v2` active pilot

## Scope

Phase 5.3 activates one Cargo Check reducer behind
`TRACEPRESS_SOURCE_REDUCER=cargo_check_v2_active`. It remains opt-in. No hook, PATH interception,
pipeline, redirection, compound shell syntax, `clippy`, search, or Git reducer participates.

## Behavioral correction

The first active smoke used `cargo_check_v1`, which removed every `Finished` line. Source reduction
was material, but the agent requested recovery because the compact output lacked an explicit final
success status. The pilot stopped after one pair.

`cargo_check_v2` retains Cargo's final `Finished` line while continuing to omit `Compiling`,
`Checking`, `Downloading`, and `Downloaded` progress. Warnings, errors, source locations, stdout,
exit status, and signals remain unchanged. V2 returned to Shadow for a complete ten-pair gate before
active evaluation.

## Active contract

An accepted candidate includes the exact recovery hint in both byte and estimated-token
never-worse comparisons. Raw stdout and stderr are stored successfully before any candidate is
emitted. Missing session, token failure, store failure, size bound, invalid UTF-8, non-applicability,
or never-worse rejection returns raw output.

The existing recovery contract remains random, opaque, same-session, one-hour expiring, bounded to
8 MiB, and byte-faithful. Telemetry never persists its token or payload.

## Paired evaluation

The success cohort runs `cargo check`. The diagnostic cohort runs an allowlisted standalone Cargo
Check invocation against a deliberately absent package. Both use the same pinned public ripgrep
commit, exact outcome sentinels, alternating session-level assignment, clean worktrees, isolated
Cargo targets, and ten pairs.

Success requires lower aggregate uncached input and a negative paired median, unchanged objective
quality, no material tool-call/retry growth, low recovery, and material source reduction. The
diagnostic cohort is a fail-open safety gate: raw output must be preserved, task outcome must remain
correct, and no recovery may occur.
