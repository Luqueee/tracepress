# Tracepress Source Policy Consolidation 012

## Decision

**PASS Phase 5.9 consolidation.**

The accepted active policies remain `cargo_test_v1_active`, `cargo_check_v2_active`, and
`rg_v1_active`, each behind explicit session opt-in. `cargo_check_v1_active` and
`git_status_v1_active` are rejected by the runtime policy registry and cannot select an active
reducer. `cargo_clippy_v1_shadow` remains negative evidence and never gained an active path.

## Safety evidence

- rejected policy selection returns the exact stdout and stderr bytes;
- rejected, unknown, and family-mismatched policies produce explicit fail-open reasons;
- policy metadata contains no command, arguments, cwd, output, prompt, provider body, or recovery
  token;
- recovery remains required before any accepted active candidate can become agent-visible;
- passthrough remains the default when no policy is selected.

## Observatory evidence

The additive comparison endpoint returns one final bounded result for each measured family:

| Family | Final evidence | Decision |
|---|---|---|
| Cargo Test | Active N=10 paired | Pass |
| Cargo Check | V2 active success N=10 paired | Pass |
| Cargo Clippy | Shadow smoke | Reject |
| ripgrep | Parseable active N=10 paired | Pass |
| Git Status | Dirty active N=10 paired | Reject |

The existing preferred-report endpoint remains compatible and continues to select the accepted
`rg_v1_active` cohort. The UI now places the five-family comparison above that detailed report.

## Scope

This phase introduces no new reducer, command parser, shell syntax, provider run, recovery format,
or global activation. Historical measurements are projected as committed evidence and are not
recalculated or combined into a synthetic savings claim.

## Validation

- workspace tests: 622 passed, 1 ignored;
- documentation tests: 4 passed;
- Clippy: all targets, all features, locked, warnings denied;
- MSRV: Rust 1.88 workspace/all-target/all-feature check passed;
- dashboard: release WASM build passed;
- formatting, build, diff, and privacy pattern checks passed.

`cargo-nextest` and `cargo-hack` were not installed in the local environment. Their pinned CI jobs
remain the authoritative execution surfaces; the equivalent workspace suite ran through
`cargo test` locally.
