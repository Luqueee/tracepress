# Tracepress explicit source-tool experiment

Decision: **REJECT**.

Control: `ExplicitGitStatusActiveControl`. Treatment: `ExplicitGitStatusActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 48240 | 48195 | -45 |
| `provider_usage.input_cached` | 44544 | 44544 | 0 |
| `provider_usage.input_uncached` | 3696 | 3651 | -45 |
| `provider_usage.output` | 186 | 131 | -55 |
| `provider_usage.reasoning` | 58 | 53 | -5 |
| `duration_ms` | 12071 | 10750 | -1321 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 453 | 453 |
| Emitted source bytes | 453 | 364 |

Invariant gate: **true**.
Source output reduction: **19.65%**.
Active gate: **false**.
Positive gate: **pending full cohort**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
