# Tracepress Shadow Compression 002

Status: **candidate selected for Phase 4.2 design; no request rewriting enabled**.

Phase 4.0 remains frozen at `93ffe0c9c32a0f9a78027c6159e70236b2e6a598` (`phase-4.0-complete`).
The complete machine-readable and generated artifacts are `shadow_pilot_002.json` and
`shadow_pilot_002.md`.

## Directed pilot

`shadow-pilot-002` ran ten isolated sessions through the real proxy path against a deterministic
local upstream: six homogeneous ToolResult JSON sessions and four repetitive ToolResult PlainText
sessions. It produced ten provider requests, ten complete snapshots, and 72 candidate evaluations.

| Integrity metric | Result |
|---|---:|
| Forwarding mutations | 0 |
| Shadow drops | 0 |
| Recovery failures | 0 |
| Determinism failures | 0 |
| Unknown transformed | 0 |

## Candidate evidence

| Candidate | Applicable | Byte reduction | Recovery | Determinism | P95 |
|---|---:|---:|---:|---:|---:|
| `json.tabular` | 6/6 | 41.04% | 100% | 100% | 2.775 ms |
| `text.repeated_line` | 4/4 | 47.06% | 100% | 100% | 0.310 ms |

`json.tabular` is the single selected candidate for a future Phase 4.2 evaluation because it is
the primary scoped target and has material directed reduction with bounded, deterministic,
recoverable output. `text.repeated_line` remains characterized but is not selected concurrently.

The selection is not a provider-token, cache, cost, or quality claim. The directed payloads are
synthetic; naturalistic Pilot 001 showed only a 0.12% workload-concentrated tabular signal. Phase
4.2 must therefore validate decode/edit/re-encode behavior with session-level A/B and keep the
forwarding byte-exact control arm.

`UnknownTransformPolicy = Never` remains enforced. Candidate and recovery content were not
persisted. Full metadata is available in [shadow_pilot_002.json](./shadow_pilot_002.json).

## Next gate

Phase 4.2 is ready for design, not active traffic. Its first experiment must use only
`json.tabular`, preserve the original forwarding arm, and compare provider observations without
conflating local representation reduction with provider caching.
