# TRACEPRESS ACTIVE COMPRESSION PROVIDER A/A 001

Authenticated infrastructure gate for the explicit `json.minify` arm. No request body was
persisted in this report.

| Arm | Codex invocations | Provider requests | Encoding | Analysis | Attempts | Rewrites | Recovery / determinism |
|---|---:|---:|---|---:|---:|---:|---:|
| Control reference (`off`) | 1 | 2 | `zstd` | 2/2 | 0 | 0 | — / — |
| Active (`json.minify`) | 2 | 4 | `zstd` | 4/4 | 4 | 0 | 0 / 0 |

The active arm reached the authenticated provider path and completed analysis for all four
requests. Both attempts per invocation were `NoImprovement` or `NotApplicable`; no candidate was
forwarded. The default/control path therefore remains byte-exact in this cohort.

This is not a valid paired provider A/A comparison because the active and control references have
different invocation/request cardinality. It is retained as an infrastructure result only. It does
not establish provider-token, cache, cost, quality, or causal savings impact. The non-fatal Codex
`/v1/models` 404 remains outside the adapter.

Runtime commit: `5c0221046d1d8bed3bc6d1e0e804b5f192612985`.
