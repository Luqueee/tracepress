# TRACEPRESS ACTIVE COMPRESSION PROVIDER A/A 002

Cardinality-matched authenticated infrastructure gate for the explicit `json.minify` arm.
No prompt, tool-result, response, header, or URL content is persisted in this report.

- Runtime commit: `99e33b42a1a7ac8f0cfb72c261783428c7265ea6`
- Workload: `directed_tool_result_json_v1`; N=10 per arm
- Provider: authenticated ChatGPT Codex subscription

| Arm | Provider requests | Analysis | Encoding | Attempts | Rewrites | Recovery | Determinism | Errors |
|---|---:|---:|---|---:|---:|---:|---:|---:|
| Control (`off`) | 20 | 20/20 | zstd | 0 | 0 | — | — | 0 |
| Active (`json.minify`) | 20 | 20/20 | zstd | 20 | 0 | 0 | 0 | 0 |

Per-invocation request cardinality: control `[2, 2, 2, 2, 2, 2, 2, 2, 2, 2]`; active `[2, 2, 2, 2, 2, 2, 2, 2, 2, 2]`.
Cardinality matched: `true`.

The active adapter reached the provider path and retained the original bytes for every
request because candidates were `NoImprovement` or `NotApplicable`. This proves only
forwarding reachability and bounded failure behavior; it does not establish provider
token, cache, cost, quality, or causal savings impact.

The non-fatal Codex `/v1/models` 404 remains outside the adapter.
