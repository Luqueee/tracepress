# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckActiveControl`. Treatment: `ExplicitCheckActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 47959 | 47956 | -3 |
| `provider_usage.input_cached` | 33280 | 33280 | 0 |
| `provider_usage.input_uncached` | 14679 | 14676 | -3 |
| `provider_usage.output` | 212 | 231 | 19 |
| `provider_usage.reasoning` | 84 | 103 | 19 |
| `duration_ms` | 10840 | 10849 | 9 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 307 | 307 |
| Emitted source bytes | 307 | 307 |

Invariant gate: **true**.
Source output reduction: **0.00%**.
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment fail-open preserved raw output; downstream deltas are safety A/A evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
