# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitRgActiveControl`. Treatment: `ExplicitRgActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 56357 | 54000 | -2357 |
| `provider_usage.input_cached` | 44544 | 29184 | -15360 |
| `provider_usage.input_uncached` | 11813 | 24816 | 13003 |
| `provider_usage.output` | 140 | 178 | 38 |
| `provider_usage.reasoning` | 47 | 60 | 13 |
| `duration_ms` | 10948 | 16896 | 5948 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 28878 | 28878 |
| Emitted source bytes | 28878 | 20027 |

Invariant gate: **true**.
Source output reduction: **30.65%**.
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
