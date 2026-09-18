# Phase 5.7.1: Tool Proxy modularization

## Scope

Phase 5.7.1 is a behavior-preserving structural refactor between the accepted `git_status_v1`
Shadow cohort and its active pilot. It changes no command admission, reducer policy, emitted bytes,
metadata schema, environment variable, recovery behavior, public function name, or default runtime
configuration.

## Module boundaries

`tracepress-tool-proxy` remains one crate with one stable public facade:

```text
src/
├── lib.rs
├── admission.rs
├── codex_hook.rs
├── execution.rs
├── model.rs
├── reducers.rs
└── reducers/
    ├── cargo_diagnostics.rs
    ├── cargo_test.rs
    ├── git_status.rs
    └── ripgrep.rs
```

- `lib.rs` declares modules and reexports the existing API.
- `model.rs` owns public contracts and backwards-compatible aliases.
- `admission.rs` owns conservative command and shell-syntax decisions.
- `codex_hook.rs` owns Codex hook parsing and rewrite responses.
- `execution.rs` owns process execution, exit codes, signals, and byte counts.
- `reducers.rs` is the private reducer registry and shared token estimator.
- each reducer module owns pure transformation logic and colocated contract tests.

The repository forbids `mod.rs`, so the registry uses `reducers.rs` with explicit paths to the
family modules.

## Deliberate non-goals

This phase does not introduce reducer traits, dynamic plugins, per-reducer crates, typed metric
variants, or a new runtime abstraction. `SourceOutputCandidate` remains unchanged to preserve its
public contract. Source recovery, persistence, and active/shadow orchestration remain in the CLI;
extracting those functions from `main.rs` is a separate refactor boundary and is not required before
the next experiment.

## Equivalence gate

Closure requires unchanged public imports from the CLI, all existing reducer/hook/admission tests,
workspace-wide tests, workspace Clippy with warnings denied, a real source-tool smoke, clean diff
checks, and no experiment-report mutation.
