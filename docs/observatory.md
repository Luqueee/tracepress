# Tracepress Observatory

Tracepress Observatory is a local-first, read-only Dioxus web interface over the operational Tracepress SQLite database. It is an investigation tool, not a hosted product.

## Run it

Build the frontend once, then start the API and static asset server:

```bash
cd crates/tracepress-dashboard
dx build --platform web --release
cd ../..
cargo run -p tracepress-cli -- ui
```

Open `http://127.0.0.1:4319`. The command binds loopback only. Phase 4.0 rejects non-loopback binds rather than exposing an unauthenticated local database browser.

For UI work without Codex or user data:

```bash
cargo run -p tracepress-cli -- ui --fixture
```

The large-data smoke fixture is also synthetic:

```bash
cargo run -p tracepress-cli -- ui --fixture --large-fixture
```

It contains 1,000 sessions, 10,000 requests, and 100,000 metadata-only context blocks. Temporary fixture databases are created outside the repository and deleted when the server exits.

Set `TRACEPRESS_DASHBOARD_ASSETS` to a compiled Dioxus `public` directory when assets are packaged elsewhere. Set `TRACEPRESS_REPORTS` to the repository `reports` directory when starting outside the repository root. Neither setting grants write access.

## Development setup

Use two terminals for fast frontend iteration:

```bash
# terminal 1, repository root
cargo run -p tracepress-cli -- ui --fixture

# terminal 2
cd crates/tracepress-dashboard
dx serve --platform web
```

`Dioxus.toml` proxies `/api/` to `http://127.0.0.1:4319/api/`. Dioxus 0.7 discovers `tailwind.css`, compiles it into `assets/tailwind.css`, and watches the Rust/CSS sources. The application also keeps its small token-based base stylesheet local, so it loads no CDN, remote font, script, or telemetry. The WASM tracing logger reports without console color styles, so Dioxus diagnostics do not trigger inline-style CSP violations.

## Architecture

```text
Browser
  -> Dioxus 0.7 WASM
  -> typed HTTP /api/v1
  -> tracepress-dashboard-api (Axum)
  -> SQLite opened READ_ONLY + PRAGMA query_only
```

The crates have separate responsibilities:

- `tracepress-dashboard-types` contains serde DTOs only and compiles natively and for `wasm32-unknown-unknown`. It has no SQLite, Axum, Tokio, filesystem, or Tracepress domain dependency.
- `tracepress-dashboard-api` owns bounded prepared queries, safe error mapping, baseline report projections, and synthetic fixture generation.
- `tracepress-dashboard` is the Dioxus WASM application and its SVG charts/design primitives.
- `tracepress-cli` supplies `tracepress ui`, keeps the fixture lifetime scoped to the process, and serves built assets beside the API.

The daemon remains the only operational database writer. Observatory does not call proxy, scheduler, provider parser, context parser, or compression runtime paths.

## Read-only and privacy guarantees

Every API query opens a short-lived SQLite connection with both `SQLITE_OPEN_READ_ONLY` and `PRAGMA query_only = ON`. There is no migration path, generic query builder, arbitrary SQL endpoint, mutation route, or long-lived read transaction. A test attempts an `INSERT` through this connection and verifies that SQLite rejects it.

The API uses explicit dashboard DTOs rather than serializing storage/provider/context domain structs. Responses contain metrics and classification metadata only. They never include prompt or assistant content, tool-result content, tool arguments, raw request/response bodies, raw URLs, headers, cookies, secrets, or CAS content. Context fingerprints are truncated to ten hexadecimal characters plus an ellipsis.

Missing provider and estimator measurements remain JSON `null` and render as `—`, `Unavailable`, or `Not comparable`; they never silently become zero. Every metric carries provenance (`provider_reported`, `locally_estimated`, or `unavailable`). Estimator coverage is shown next to composition views.

## API v1

Operational SQLite endpoints:

```text
GET /api/v1/overview
GET /api/v1/sessions
GET /api/v1/sessions/:id
GET /api/v1/sessions/:id/requests
GET /api/v1/sessions/:id/context
GET /api/v1/context/composition
GET /api/v1/context/repetition
GET /api/v1/context/unknown
GET /api/v1/health
```

Artifact exploration endpoints:

```text
GET /api/v1/workloads
GET /api/v1/baselines
GET /api/v1/baselines/:id
GET /api/v1/opportunities
GET /api/v1/compression/experiments
GET /api/v1/compression/experiments/:id
GET /api/v1/compression/experiments/:id/candidates?limit=50&cursor=...
```

Sessions accept `limit`, opaque `cursor`, `model`, `transport`, `status`, `has_compaction`, `from`, `to`, `sort`, and `direction`. Page size is restricted to 1–100. `workload` and `measurement_id` are accepted contract fields, but return no operational matches until a certified operational mapping exists; Observatory does not borrow labels from unrelated reports. Supported sorts are `created_at`, `request_count`, `input_tokens`, `cache_ratio`, `estimated_context_tokens`, and `repetition`.

The baseline/workload/opportunity endpoints read reproducible JSON report artifacts. They do not replace or modify those artifacts, and official convergence status is projected rather than recalculated.

Compression Lab reads migration 0009 metadata from operational SQLite. It shows experiment quality,
compressor applicability, local byte/token-estimate reduction, recovery, determinism, latency,
prefix evidence, cache risk, and paginated block metadata. It never returns original or candidate
content. Workload distributions remain explicitly unavailable until operational sessions carry a
certified workload mapping.

Generate the reproducible metadata-only experiment report with:

```bash
python3 scripts/analyze_shadow_compression.py \
  --db "$TRACEPRESS_DB" \
  --experiment-id shadow-pilot-001 \
  --manifest reports/shadow-compression-001/experiment_manifest.json \
  --output-json reports/shadow-compression-001/shadow_pilot_n10.json \
  --output-md reports/shadow-compression-001/shadow_pilot_n10.md
```

The analyzer opens SQLite with URI `mode=ro` plus `PRAGMA query_only`, emits no fingerprints or
content, and leaves missing estimates as JSON `null`. The manifest is the auditable source for
workload labels, empirical gate decisions, and the single candidate decision. Passing gates alone
never causes the analyzer to select a candidate automatically.

The directed follow-up `Shadow Pilot 002` is recorded under
`reports/shadow-compression-002/`. It selected `json.tabular` as the single candidate for Phase
4.2 evaluation design; this is local representation evidence only, not provider-token or quality
evidence. Real-provider request rewriting remains disabled until its decode/edit/re-encode A/B
control is explicitly implemented and validated.

The first explicit Phase 4.2 adapter is now available for an infrastructure-only A/B smoke. It is
disabled by default and can be enabled only alongside complete context analysis:

```bash
TRACEPRESS_CONTEXT_ANALYSIS=shadow \
TRACEPRESS_ACTIVE_COMPRESSION=json.minify \
tracepress run <agent> ...
```

The adapter rewrites only complete `ToolGenerated + ToolResult + Json` spans, removes structural
JSON whitespace, verifies deterministic output and exact recovery, and removes a stale
`Content-Length` before forwarding the shorter body. Any malformed, unsupported-encoding, partial,
or resource-limited request fails open to the original bytes. zstd requests use bounded
decode/edit/re-encode; the `Content-Encoding: zstd` header is retained. `json.tabular` remains
shadow-only: its
`TPJ2` representation is not a provider-compatible Responses payload and is not sent upstream.
Active metrics are metadata-only (`original`/`rewritten` sizes and fingerprints) and are offered to
the observation sink without blocking it. The active counters also expose bounded evaluated-span
and candidate byte totals for `NoImprovement` diagnostics, never the underlying content. The
explicitly enabled active arm performs bounded
analysis and rewriting before its upstream `send`; the default path remains byte-exact and does
not pay that cost. This adapter does not establish provider-token, cache, cost, or quality impact;
those require the Phase 4.2 A/B experiment and provider-side validation.

The reproducible local infrastructure smoke is driven by
`scripts/run_active_compression_pilot.py` and recorded in
`reports/active-compression-001/TRACEPRESS_ACTIVE_COMPRESSION_002.{json,md}`. It runs a control and
an active arm through isolated daemons against a deterministic local upstream; its 20.1% median
forwarded-byte reduction is local representation evidence only. The earlier `001` artifact is
retained as the historical run from the pre-zstd-metric runtime.

The authenticated provider follow-ups are recorded separately. The earlier
`TRACEPRESS_ACTIVE_COMPRESSION_PROVIDER_AA_001.{json,md}` artifact is retained as a blocked
attempt because active and control request cardinality was not matched. The cardinality-matched
cohort `TRACEPRESS_ACTIVE_COMPRESSION_PROVIDER_AA_002.{json,md}` runs ten invocations per arm
(20 provider requests each), with complete analysis and zero request, recovery, or determinism
failures. It accepted no active rewrite, so it remains infrastructure evidence rather than
provider-impact evidence. The follow-up diagnostic
`TRACEPRESS_ACTIVE_COMPRESSION_PROVIDER_DIAGNOSTIC_001.{json,md}` records why: one eligible span
was evaluated at 224 input bytes and 224 candidate bytes, so the never-worse guard kept it.

Errors have a browser-safe shape:

```json
{
  "error": {
    "code": "session_not_found",
    "message": "Session was not found"
  }
}
```

Raw SQLite errors, stack traces, and filesystem paths are not returned.

## Build and validation

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build -p tracepress-dashboard --target wasm32-unknown-unknown --release --locked
cd crates/tracepress-dashboard && dx build --platform web --release
```

Release deployment consists of the `tracepress` binary and the Dioxus `public` directory; no Node/npm process is required at runtime. The CLI discovers the standard local Dioxus output or accepts its packaged location through `TRACEPRESS_DASHBOARD_ASSETS`.

## Security headers and refresh model

The Axum router applies Content Security Policy, `X-Content-Type-Options: nosniff`, and `Referrer-Policy: no-referrer`. CSP keeps stylesheets on the same origin with `style-src 'self'` and grants `style-src-attr 'unsafe-inline'` narrowly because Dioxus updates runtime style attributes used by SVG chart geometry. This avoids weakening `style-src` for inline style elements. Its script policy includes `unsafe-eval` because the verified Dioxus 0.7 WASM bootstrap uses dynamic function construction; the server remains loopback-only and loads no remote scripts. Phase 4.0 uses manual browser refresh; it has no WebSocket, account system, authentication, remote access, analytics, or external telemetry.

The detailed visual rules and measurement language live in [the Observatory design system](design/observatory-design-system.md).
