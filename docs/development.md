# Development checks

Tracepress keeps fast correctness checks on every change and moves expensive ecosystem checks to
dedicated CI jobs. The commands below mirror the repository workflows.

## Required local checks

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked
cargo test --workspace --all-features --locked --doc
cargo build --workspace --all-features --locked
cargo +1.88.0 check --workspace --all-targets --all-features --locked
cargo hack check -p tracepress-core --each-feature --locked
cargo hack check -p tracepress-dashboard --each-feature --locked
```

`cargo-nextest` runs each test in its own process and produces a JUnit report in CI. It does not run
documentation tests, so the explicit `cargo test --doc` command is a required, separate gate.
Rust 1.88 is the minimum supported version and has its own compile job so dependency updates cannot
silently raise it.

The forwarding-path latency regressions and the bounded-analysis deadline regression reserve the
complete nextest thread pool and run last. Their assertions measure forwarding behavior or a fixed
work budget, so competing test processes must not supply the load being measured; this is resource
isolation, not a retry or relaxed threshold.

## Coverage

Install `cargo-llvm-cov` and generate the same formats used in CI:

```console
cargo llvm-cov --workspace --all-features --locked --no-report
cargo llvm-cov report --workspace --summary-only --fail-under-lines 80
cargo llvm-cov report --workspace --html
```

CI publishes the text summary and LCOV data as a build artifact. The current whole-workspace baseline
is 81.96% line coverage, so CI enforces a conservative 80% floor. Raise the threshold as coverage
improves; do not lower it to merge untested behavior.

## Dependency policy

```console
cargo deny check
cargo machete --with-metadata
```

`cargo-deny` gates pull requests that change dependency metadata and checks RustSec advisories,
licenses, duplicate versions, and dependency sources. `cargo-machete` is intentionally scheduled or
manual because its text-based analysis can report false positives; review its findings before
removing a dependency.

Dependabot keeps both Cargo dependencies and the immutable GitHub Actions commit references current.
Every action reference is pinned to a reviewed full commit SHA; the trailing version comment is only
for maintainers and Dependabot.

## Slow checks

The bounded fuzz workflow runs weekly and can also be started manually. Keep property tests with the
normal test suite; add fuzz targets for parsers and other untrusted-input boundaries where a bounded
randomized test can assert a concrete invariant.

Mutation testing is manual because the three initial target packages expose more than 1,400 possible
mutants. Start the Mutation testing workflow with one package and one deterministic shard. The
workflow verifies the unmodified package before running cargo-mutants, and always retains its report.

## Deferred browser end-to-end tests

Dashboard browser E2E is outside the current maintenance scope. The dashboard currently has native
SSR/component tests but no browser harness. When dashboard work is resumed explicitly, add browser
E2E only with a repository-owned setup that can build the Dioxus WASM bundle, start
`tracepress ui --fixture` on an ephemeral loopback port, wait on `/api/v1/health`, and retain browser
diagnostics on failure. The first scenarios should cover overview load, session navigation,
unavailable-value rendering, and an API failure state. Tests must use synthetic fixtures and must
not read the operational database.

Do not add Playwright or another JavaScript dependency solely to collect a nominal coverage number;
the harness needs to exercise the shipped WASM application and its typed HTTP boundary.

## Release automation

Continuous deployment is intentionally not configured yet. Before adding a release workflow, decide
and document the version/tag contract, supported target matrix, dashboard-asset packaging, archive
layout, checksums, signing or provenance, and rollback/withdrawal procedure. The workflow must build
from an immutable tag, rerun release gates, use least-privilege permissions, and must not publish from
a pull-request context.

The packages declare `MIT OR Apache-2.0`; add both license text files before producing a public
release artifact.
