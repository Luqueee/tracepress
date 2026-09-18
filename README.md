# Tracepress

Tracepress is a local-first Rust runtime for observing AI-agent traffic, measuring explicit and
provider-managed context, and evaluating bounded context-reduction candidates. Its operational
path is designed to fail open: observation or analysis failures must not prevent the original
request from being forwarded.

The workspace includes an authenticated local IPC layer, a single-writer SQLite store, provider
and context analysis, an HTTP proxy, an explicit source-tool boundary, and a read-only Dioxus
Observatory. Analysis records metadata and measurements rather than prompt, response, tool-result,
credential, or raw provider-body content.

Tracepress is pre-1.0 research software. Active reductions are opt-in and each experiment document
states what its evidence does and does not establish.

## Requirements

- Rust 1.88 or newer (the workspace's minimum supported Rust version)
- Dioxus CLI only when building the Observatory web assets

The pinned toolchain in `rust-toolchain.toml` is the normal development toolchain. CI separately
checks the minimum supported version.

## Quick start

Build the workspace and inspect the CLI:

```console
cargo build --workspace --all-features --locked
cargo run -p tracepress-cli --bin tracepress -- --help
```

Initialize local state, validate it, and run an agent through Tracepress:

```console
cargo run -p tracepress-cli --bin tracepress -- init
cargo run -p tracepress-cli --bin tracepress -- doctor
cargo run -p tracepress-cli --bin tracepress -- run --help
```

Tracepress stores local runtime state outside the repository. Set `TRACEPRESS_HOME` to an isolated
directory for development or tests that must not inspect normal user state.

To launch the read-only Observatory with synthetic data, first build its web assets and then start
the loopback-only server:

```console
cd crates/tracepress-dashboard
dx build --platform web --release
cd ../..
cargo run -p tracepress-cli --bin tracepress -- ui --fixture
```

See [docs/observatory.md](docs/observatory.md) for the operational dashboard, API, privacy model,
and larger synthetic fixture.

## Development

[CONTRIBUTING.md](CONTRIBUTING.md) explains the contribution workflow. The exact local equivalents
of CI, coverage, dependency-policy, feature-combination, fuzzing, and mutation checks live in
[docs/development.md](docs/development.md). The ordered maintenance backlog and explicitly deferred
work are recorded in [docs/next-steps.md](docs/next-steps.md).

Architecture and experiment contracts are under `docs/design/` and reproducible research summaries
are under `docs/research/`. Reports must state their evidence boundary; local byte reduction is not
automatically evidence of provider-token, cache, cost, quality, or end-to-end task improvement.

## Security

Tracepress handles local credentials and potentially sensitive agent traffic. Please read
[SECURITY.md](SECURITY.md) before reporting a vulnerability and do not put secrets, prompt content,
or exploit details in a public issue.

## License

Workspace packages declare `MIT OR Apache-2.0`. The repository must include the corresponding
license text files before a public distribution or release artifact is published.
