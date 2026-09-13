# TRACEPRESS ACTIVE COMPRESSION 002

Infrastructure-only A/B smoke for the explicitly enabled `json.minify` adapter.

- Runtime commit: `5c0221046d1d8bed3bc6d1e0e804b5f192612985`
- Workload: `tool_result_json`; N=10
- Upstream: deterministic local HTTP/1.1 server

| Arm | Median forwarded bytes | Active attempts | Rewrites | Recovery failures | Determinism failures |
|---|---:|---:|---:|---:|---:|
| Control | 298.0 | 0 | 0 | — | — |
| Active | 238.0 | 11 | 11 | 0 | 0 |

Median local forwarded-byte reduction: `0.20134228187919462`.
This is representation evidence from a local upstream only; it is not provider-token, cache, cost, or quality evidence.

The deterministic performance smoke completed with zero request errors. Active median dispatch was `634.460 µs` versus `577.120 µs` for control; median total duration was `3975.504 µs` versus `3992.489 µs`. Peak RSS was `7808 KiB` active versus `7368 KiB` control (a `440 KiB` arm delta), and child CPU was `100911 µs` versus `105595 µs`. These are process-lifecycle measurements on a local upstream, not a production performance SLO.

The default (`TRACEPRESS_ACTIVE_COMPRESSION=off`) remains byte-exact. `json.tabular` is not sent upstream because its TPJ2 representation has no provider decoding contract.
