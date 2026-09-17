# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckV2Control`. Treatment: `ExplicitCheckV2Active`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 21 | 0.0 |
| `provider_usage.input_total` | 479262 | 503876 | 25.5 |
| `provider_usage.input_cached` | 432128 | 447232 | 0.0 |
| `provider_usage.input_uncached` | 47134 | 56644 | 591.0 |
| `provider_usage.output` | 2075 | 2240 | 8.5 |
| `provider_usage.reasoning` | 940 | 1006 | 11.0 |
| `duration_ms` | 115353 | 120641 | -13.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 11 |
| Command retries | 0 | 1 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 3070 | 3377 |
| Emitted source bytes | 3070 | 3377 |

Invariant gate: **true**.
Source output reduction: **0.00%**.
Active gate: **true**.
Positive gate: **true**.
Treatment fail-open preserved raw output; downstream deltas are safety A/A evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
