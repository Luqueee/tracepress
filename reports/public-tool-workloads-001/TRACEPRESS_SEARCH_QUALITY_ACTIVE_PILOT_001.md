# TRACEPRESS_SEARCH_QUALITY_ACTIVE_PILOT_001

Bounded public Search Control/Treatment pilot using the pinned ripgrep workload.

Reducer: `search.result_projection` v1. Model: `gpt-5.6-luna`. Sessions: `12`.

| Arm | Task | Success | Input | Uncached | Rewrites | Evaluator |
|---|---|---:|---:|---:|---:|---|
| `control` | `search-01` | true | — | — | 0 | success |
| `control` | `search-02` | false | 19133 | 957 | 0 | unavailable |
| `control` | `search-03` | true | 38847 | 10687 | 0 | success |
| `control` | `search-04` | false | 19133 | 9149 | 0 | unavailable |
| `control` | `search-05` | true | 39840 | 3488 | 0 | success |
| `control` | `search-06` | true | 38567 | 10407 | 0 | success |
| `treatment` | `search-01` | true | — | — | 0 | success |
| `treatment` | `search-02` | false | 19133 | 957 | 0 | unavailable |
| `treatment` | `search-03` | true | 38964 | 10804 | 0 | success |
| `treatment` | `search-04` | false | 44162 | 7810 | 0 | failure |
| `treatment` | `search-05` | true | 39519 | 3167 | 0 | success |
| `treatment` | `search-06` | true | 38548 | 10388 | 0 | success |

Forwarding mutations: **0**. Recovery failures: **0**. Determinism failures: **0**.

Active candidate evaluations: **0**; active rewrites: **0**. Treatment quality is not comparable because the candidate was never exercised; this run is an infrastructure diagnostic, not an efficacy result.

This pilot is limited to the pinned public workload. It does not establish universal quality, provider savings, cache causality, or economic impact.

Privacy audit: **metadata-only report**; no raw task or provider content persisted.
