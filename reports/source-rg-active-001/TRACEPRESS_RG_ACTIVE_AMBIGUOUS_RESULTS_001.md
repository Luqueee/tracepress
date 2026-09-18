# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitRgActiveControl`. Treatment: `ExplicitRgActive`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 566115 | 577114 | 217.5 |
| `provider_usage.input_cached` | 381952 | 445440 | 0.0 |
| `provider_usage.input_uncached` | 184163 | 131674 | -7487.0 |
| `provider_usage.output` | 1518 | 1608 | 6.5 |
| `provider_usage.reasoning` | 503 | 559 | 10.5 |
| `duration_ms` | 110712 | 120601 | 31.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 2282620 | 2282620 |
| Emitted source bytes | 2282620 | 2282620 |

Invariant gate: **true**.
Source output reduction: **0.00%**.
Active gate: **true**.
Positive gate: **true**.
Treatment fail-open preserved raw output; downstream deltas are safety A/A evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
