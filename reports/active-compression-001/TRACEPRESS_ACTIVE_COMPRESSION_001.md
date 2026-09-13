# TRACEPRESS ACTIVE COMPRESSION 001

Infrastructure-only A/B smoke for the explicitly enabled `json.minify` adapter.

- Runtime commit: `a225cbcf277d4c1ba3324bc3847a5cf0f4e16b27`
- Workload: `tool_result_json`; N=10
- Upstream: deterministic local HTTP/1.1 server

| Arm | Median forwarded bytes | Active attempts | Rewrites | Recovery failures | Determinism failures |
|---|---:|---:|---:|---:|---:|
| Control | 298.0 | 0 | 0 | — | — |
| Active | 238.0 | 11 | 11 | 0 | 0 |

Median local forwarded-byte reduction: `0.20134228187919462`.
This is representation evidence from a local upstream only; it is not provider-token, cache, cost, or quality evidence.

The default (`TRACEPRESS_ACTIVE_COMPRESSION=off`) remains byte-exact. `json.tabular` is not sent upstream because its TPJ2 representation has no provider decoding contract.
