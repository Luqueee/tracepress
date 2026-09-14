# TRACEPRESS ACTIVE COMPRESSION PROVIDER DIAGNOSTIC 001

Metadata-only follow-up explaining the `NoImprovement` result on a real authenticated request.

| Metric | Value |
|---|---:|
| Provider requests | 2 |
| Analysis complete | 2/2 |
| Content encoding | `zstd` |
| Active attempts | 2 |
| Eligible spans | 1 |
| Evaluated input bytes | 224 |
| Evaluated candidate bytes | 224 |
| Rewrites | 0 |
| No improvement | 1 |
| Not applicable | 1 |
| Recovery failures | 0 |
| Determinism failures | 0 |
| Provider errors | 0 |

The single eligible span produced an equal-size candidate, so the never-worse guard correctly kept
the original request. The second request had no applicable target. This is diagnostic evidence for
the real provider representation only; it does not expose content and does not establish provider
token, cache, cost, quality, or savings impact.

Runtime commit: `31abb4d79764e67914aac48a17820a41c7b140ed`.
