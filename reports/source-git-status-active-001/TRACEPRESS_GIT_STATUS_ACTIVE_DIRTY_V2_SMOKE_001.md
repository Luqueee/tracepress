# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitGitStatusActiveControl`. Treatment: `ExplicitGitStatusActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 48235 | 48202 | -33 |
| `provider_usage.input_cached` | 44544 | 44544 | 0 |
| `provider_usage.input_uncached` | 3691 | 3658 | -33 |
| `provider_usage.output` | 132 | 142 | 10 |
| `provider_usage.reasoning` | 40 | 61 | 21 |
| `duration_ms` | 10786 | 10806 | 20 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 453 | 453 |
| Emitted source bytes | 453 | 345 |

Invariant gate: **true**.
Source output reduction: **23.84%**.
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
