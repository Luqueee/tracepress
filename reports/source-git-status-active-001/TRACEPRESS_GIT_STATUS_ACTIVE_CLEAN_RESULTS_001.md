# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitGitStatusActiveControl`. Treatment: `ExplicitGitStatusActive`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 481091 | 481151 | 15.0 |
| `provider_usage.input_cached` | 428032 | 423936 | 0.0 |
| `provider_usage.input_uncached` | 53059 | 57215 | 22.5 |
| `provider_usage.output` | 1332 | 1338 | 10.0 |
| `provider_usage.reasoning` | 441 | 505 | 8.5 |
| `duration_ms` | 115297 | 122748 | 1.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 670 | 670 |
| Emitted source bytes | 670 | 670 |

Invariant gate: **true**.
Source output reduction: **0.00%**.
Active gate: **true**.
Positive gate: **true**.
Treatment fail-open preserved raw output; downstream deltas are safety A/A evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
