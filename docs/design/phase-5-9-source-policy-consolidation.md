# Phase 5.9 source policy consolidation

Phase 5.9 consolidates measured Phase 5 evidence. It does not add a command family, change the
default runtime policy, or claim savings beyond the existing bounded workloads.

## Policy contract

`tracepress-tool-proxy` owns the typed source reducer registry. Every known reducer has one command
family, one mode (`Shadow` or `Active`), and one measured decision:

| Reducer | Decision |
|---|---|
| `cargo_test_v1_active` | Accepted opt-in |
| `cargo_check_v2_active` | Accepted opt-in |
| `rg_v1_active` | Accepted opt-in |
| `cargo_check_v1_active` | Rejected |
| `git_status_v1_active` | Rejected |
| `cargo_clippy_v1_shadow` | Rejected before active |

Shadow policies remain available only for evidence reproduction. A rejected active policy can no
longer select an active reducer through `TRACEPRESS_SOURCE_REDUCER`. Unknown reducers, family
mismatches, rejected policies, recovery-store failures, and never-worse failures emit the original
bytes.

## Session-scoped opt-in

Passthrough remains the default. An accepted reducer requires an explicit session environment:

```console
TRACEPRESS_SOURCE_SESSION_ID=<opaque-session-uuid> \
TRACEPRESS_SOURCE_REDUCER=cargo_test_v1_active \
tracepress tool cargo test
```

The same contract applies to `cargo_check_v2_active` and `rg_v1_active`. Recovery remains
session-scoped and mandatory before filtered bytes can be emitted. Phase 5.9 does not enable any
accepted policy globally or transparently.

## Observatory

`GET /api/v1/source-optimization` remains the compatible preferred-report endpoint.
`GET /api/v1/source-optimization/experiments` adds an allowlisted five-family comparison across
Cargo Test, Cargo Check, Cargo Clippy, ripgrep, and Git Status. Source reduction and downstream
provider/trajectory measurements remain separate, and rejected decisions stay visible.

## Regression gate

The combined gate covers the typed registry, active selection, exact-byte fail-open for a rejected
policy, the five-family API response, dashboard compilation, privacy-safe metadata, and the full
workspace quality suite. No provider experiment is rerun because this phase changes policy
selection and presentation, not reducer output.
