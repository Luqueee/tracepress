# Phase 5.4: `cargo_clippy_v1` Shadow

## Scope

Phase 5.4 evaluates exactly one new standalone command family: `cargo clippy`. Pipelines,
redirections, substitutions, compound commands, quoting, globs, and unknown shell syntax remain
fail-open. The reducer is available only through the explicit
`TRACEPRESS_SOURCE_REDUCER=cargo_clippy_v1_shadow` policy and never changes agent-visible bytes.

## Candidate contract

The candidate preserves stdout, warnings, errors, rendered diagnostics, source locations, notes,
help, and Cargo's final `Finished` status. It removes only `Compiling`, `Checking`, `Downloading`,
and `Downloaded` progress lines. Invalid UTF-8 is non-applicable. A hypothetical recovery hint is
included in byte and estimated-token never-worse comparisons.

This deliberately tests the lowest-risk shared Cargo projection. Warning grouping, lint
deduplication, diagnostic truncation, or machine-readable rewriting are separate lossy policies and
are not smuggled into this phase.

## Gate

The first paired smoke uses the pinned public ripgrep workload, clean per-arm worktrees and Cargo
targets, the same wrapper command, an exact outcome sentinel, and raw forwarding in both arms. A
full N=10 cohort is allowed only if the smoke demonstrates at least 20% candidate reduction while
preserving command semantics and bounded latency.
