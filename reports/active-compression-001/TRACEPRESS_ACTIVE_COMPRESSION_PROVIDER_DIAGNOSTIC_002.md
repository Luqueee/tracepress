# TRACEPRESS ACTIVE COMPRESSION PROVIDER DIAGNOSTIC 002

Metadata-only diagnostic for a targeted, structurally spaced ToolResult JSON workload. It used one
invocation per arm with the same authenticated Codex client and two `zstd` provider requests per
arm. No request or candidate content is included.

| Metric | Value |
|---|---:|
| Control provider requests | 2 |
| Active provider requests | 2 |
| Analysis complete | 2/2 in both arms |
| Active attempts | 2 |
| Eligible spans | 1 |
| Evaluated input bytes | 12,286 |
| Evaluated candidate bytes | 12,286 |
| Rewrites | 0 |
| No improvement | 1 |
| Not applicable | 1 |
| Recovery failures | 0 |
| Determinism failures | 0 |
| Resource-limit failures | 0 |
| Internal errors | 0 |
| Provider errors | 0 |
| Forwarding mutations | 0 |

The targeted span produced an equal-size `json.minify` candidate, so the never-worse guard retained
the original request. This is diagnostic evidence about this real-provider representation only; it
does not expose content and does not establish provider-token, cache, cost, quality, or savings
impact. The result is not expanded to a ten-sample cohort because the one-sample gate produced no
material local reduction to measure.

Runtime commit: `a2bcdd682634121d03d9dff0189e11cea0f82109`.
Workload: `spaced_tool_result_json_v1`.
