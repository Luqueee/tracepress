# Phase 4.2 — Active Compression Experiment (infrastructure gate)

## Boundary

The normal Tracepress path remains Phase 4.0 byte-exact. Active rewriting is a separate,
explicitly selected arm and is disabled unless the operator sets:

```text
TRACEPRESS_CONTEXT_ANALYSIS=shadow
TRACEPRESS_ACTIVE_COMPRESSION=json.minify
```

The adapter currently supports identity-encoded and bounded `zstd` `/v1/responses` requests, and
only complete `ToolGenerated + ToolResult + Json` spans returned by the bounded Phase 3 analyser.
For zstd, it decodes, rewrites, and re-encodes under the active experiment's independent resource
limits before upstream dispatch. The never-worse check is applied to the decoded candidate (the
representation the provider will parse); decoded and wire byte metrics are reported separately.
It runs only after the request has been fully buffered under the existing body bound. The original
body is retained for fail-open behavior; the rewritten body is transient and never persisted as
candidate content.

```text
original body
   │
   ├── bounded Phase 3 analysis ──► eligible metadata-only spans
   │                                  │
   │                                  ▼
   └────────────────────────────── json.minify ──► recovery + determinism gate
                                                    │
                                                    ▼
                                      explicit active upstream request
```

Any malformed, unsupported-encoding, partial, overlapping, non-deterministic, non-recoverable, or
resource-limited attempt forwards the original bytes. The active adapter removes an inbound
`Content-Length` only when bytes actually change so reqwest emits a correct length for the
replacement body. All other headers and routes keep the existing forwarding behavior.

## Candidate choice

`json.tabular` remains shadow-only. Its `TPJ2` bytes are a local representation and are not a
provider-compatible Responses `function_call_output.output` value. Sending those bytes would make
the provider/model responsible for a new decoding contract that does not exist yet. The first
active candidate is therefore `json.minify`, whose replacement remains a valid JSON document and
preserves keys, values, ordering, and string content while removing structural whitespace.

## Metadata boundary

The proxy emits `ActiveCompressionObservation` to the non-blocking observation sink. It contains
only status, compressor/version, byte counts, rewrite count, recovery/determinism flags, and
original/rewritten SHA-256 fingerprints. The request and candidate bodies never cross this sink.
The CLI aggregates the same fields into bounded run output (`active_compression_*`) for a local A/B
driver. No active result is interpreted as provider-token savings, cache preservation, cost impact,
or quality preservation.

## A/A experiment protocol

The first usable experiment is a local infrastructure smoke, not a provider efficacy claim:

1. Run a deterministic workload with `TRACEPRESS_ACTIVE_COMPRESSION=off`.
2. Repeat the same workload with `TRACEPRESS_ACTIVE_COMPRESSION=json.minify`.
3. Keep provider credentials, endpoint, model, reasoning, and agent inputs fixed.
4. Compare forwarded request bytes, provider-reported usage, response timing, errors, and tool-call
   outcomes. The active arm must show zero recovery/determinism failures and no forwarding stalls.

The existing shadow/A-A reports remain the baseline for analysis and scheduler costs. This gate does
not authorize an adaptive policy or Phase 4.2 production rollout; it only proves that one explicit,
reversible candidate can be exercised without changing the default path.

The first authenticated Codex subscription smoke is recorded in
`reports/active-compression-001/TRACEPRESS_ACTIVE_COMPRESSION_PROVIDER_SMOKE_001.md`. Codex sent
`Content-Encoding: zstd` on every request, so the identity-only adapter correctly produced zero
active attempts and forwarded the original bytes. That report is historical: the active adapter now
has a bounded zstd decode/edit/re-encode path. A subsequent smoke must still be treated as an
infrastructure check, not a provider A/B result; zstd re-encoding does not establish provider-token
savings, cache preservation, cost impact, or quality preservation.
