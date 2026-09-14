# Phase 4.1 — Shadow Compression

## Frozen Phase 4.0 base

Phase 4.0 is frozen at commit `93ffe0c9c32a0f9a78027c6159e70236b2e6a598`, tagged
`phase-4.0-complete`. Phase 4.1 is additive and must remain auditable relative to that tag.

## Invariants

- The provider receives the original `WireBody`; candidate bytes never enter the forwarding path.
- Shadow work starts only after Phase 3 analysis and durable snapshot finalization.
- `tracepressd` remains the only SQLite writer.
- Candidate and recovery bytes are ephemeral. Only status, sizes, estimates, timings, fingerprints,
  prefix evidence, recovery verification, determinism, cache-risk classification, provider
  readability, and bounded shape labels are persisted.
- `UnknownTransformPolicy` is permanently `Never` in this phase.
- Candidate reduction is local representation evidence. It is not provider token savings, a cache
  prediction, a cost claim, or a quality claim.

## Data flow and backpressure

```text
original WireBody ─────────────────────────────► provider
       │                                          unchanged
       ▼
bounded Phase 3 decode + context analysis
       │
       ▼ after durable snapshot finalization
independent shadow queue (8 jobs / 16 MiB)
       │ try_send; never awaited by forwarding
       ▼
bounded compression worker
       │ metadata-only micro-batches
       ▼
tracepressd ─► SQLite migrations 0009/0010/0011 ─► Observatory
```

The queue has independent item and byte budgets. Saturation increments `shadow_drops`; it never
borrows Phase 3 queue capacity and never delays forwarding. Migration 0010 records queue-full,
byte-budget, work-budget, worker-closed, and persistence drop reasons for new experiments. One
snapshot also has a global shadow work budget, while each compressor enforces input, output,
memory, work, wall-time, and candidate count bounds.

## Target contract

JSON compressors support only `ToolGenerated + ToolResult + Json`. Plain-text compressors support
only `ToolGenerated + ToolResult + PlainText`. Tool schemas, human and agent content,
provider-managed content, other detected kinds, and Unknown are rejected before transformation.

Implemented controls and candidates:

| ID | Transformation | Recovery |
|---|---|---|
| `json.noop` | byte-identical control | exact |
| `json.minify` | removes structural JSON whitespace without reserializing values | exact, using bounded whitespace provenance |
| `json.tabular` | deterministic homogeneous `array<object>` tuple encoding | exact, using ephemeral provenance |
| `json.repeated_subtree` | exact repeated object/array definitions and references | exact, using ephemeral provenance |
| `json.readable_table` | explicit human-readable table for homogeneous rows | exact, using ephemeral provenance |
| `json.compact_records` | explicit human-readable compact records | exact, using ephemeral provenance |
| `json.key_elision` | explicit schema header plus human-readable records | exact, using ephemeral provenance |
| `text.noop` | byte-identical control | exact |
| `text.repeated_line` | binary length/count encoding of consecutive equal lines | exact and self-contained |
| `text.repeated_run` | bounded folding of adjacent repeated line runs | exact and self-contained |
| `text.readable_line_fold` | explicit repeated-line representation | exact, using ephemeral provenance |
| `text.readable_block_fold` | explicit repeated-block representation | exact, using ephemeral provenance |
| `text.log_prefix_fold` | conservative structured-log factoring | exact, using ephemeral provenance |

Duplicate JSON keys make structural JSON candidates `NotApplicable`. Invalid UTF-8, invalid JSON,
depth/work exhaustion, expansion, and elapsed-time exhaustion have explicit typed states. A valid
candidate that is not smaller in bytes, or in comparable locally estimated tokens, is
`NoImprovement`; negative savings are never recorded.

Every reversible candidate is transformed twice and candidate fingerprints must match. Recovery
then hashes the recovered bytes and requires equality with the original SHA-256. Candidate bytes,
recovery buffers, request bodies, prompt/tool-result contents, arguments, headers, URLs, and
credentials are absent from the daemon control DTO and migration.

## Prefix evidence and cache risk

`first_modified_offset`, `preserved_prefix_bytes`, and the request-relative prefix ratio are
structural evidence. The initial `Low / Medium / High / Unknown` cache-risk label combines that
position with available persistence evidence. It is explicitly not a provider cache prediction.

## Operation

Shadow compression is opt-in while Phase 4.1 is evaluated:

```bash
TRACEPRESS_CONTEXT_ANALYSIS=shadow \
TRACEPRESS_SHADOW_COMPRESSION=on \
TRACEPRESS_SHADOW_EXPERIMENT_ID=shadow-pilot-001 \
tracepress run codex ...
```

The experiment identifier accepts 1–128 ASCII alphanumeric, `.`, `_`, or `-` characters. The
default is `shadow-pilot-001`. Turning shadow compression on while Phase 3 analysis is off is a
configuration error.

For a bounded local resource smoke, the Phase 3 harness accepts
`--shadow-compression`; it enables compression only in the context-analysis ON arm and records
the flag in the JSON manifest. The harness now also records per-arm child user/system CPU time
from `RUSAGE_CHILDREN`; this is lifecycle-inclusive observational evidence, not a host-wide CPU
SLO and never a provider-impact measurement.

## Empirical gates

Synthetic fixtures, unit/integration tests, and a controlled byte-exact local-upstream run validate
the data path, privacy boundary, recovery, determinism, pagination, and zero mutation. The
naturalistic ten-session Shadow Pilot, post-hardening infrastructure smokes, and the five-sample
resource A/A comparison are recorded in `reports/shadow-compression-001/`. The resource comparison
is observational only: it found no timing regression outside the measured A/A envelope, while
Shadow ON used 384 KiB more peak RSS and 13.95 ms more child CPU than OFF on the synthetic
ToolResult JSON workload. A larger naturalistic follow-up remains necessary before treating those
resource values as stable. Directed Pilot 002 subsequently characterized a material synthetic
signal and selected `json.tabular` for Phase 4.2 evaluation design; it remains shadow-only because
its TPJ2 bytes have no provider decoding contract. The provider-compatible `json.minify` adapter
has a separate local infrastructure smoke, but real provider traffic remains disabled pending an
explicit decode/edit/re-encode A/B gate.
