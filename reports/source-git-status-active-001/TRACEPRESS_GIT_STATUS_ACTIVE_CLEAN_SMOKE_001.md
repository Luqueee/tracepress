# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitGitStatusActiveControl`. Treatment: `ExplicitGitStatusActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 48125 | 48163 | 38 |
| `provider_usage.input_cached` | 44544 | 44544 | 0 |
| `provider_usage.input_uncached` | 3581 | 3619 | 38 |
| `provider_usage.output` | 146 | 171 | 25 |
| `provider_usage.reasoning` | 55 | 43 | -12 |
| `duration_ms` | 10805 | 10860 | 55 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 67 | 67 |
| Emitted source bytes | 67 | 67 |

Invariant gate: **true**.
Source output reduction: **0.00%**.
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment fail-open preserved raw output; downstream deltas are safety A/A evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
