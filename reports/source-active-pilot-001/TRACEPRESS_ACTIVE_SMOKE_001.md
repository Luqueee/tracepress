# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitActiveControl`. Treatment: `ExplicitActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 53526 | 48155 | -5371 |
| `provider_usage.input_cached` | 33280 | 33280 | 0 |
| `provider_usage.input_uncached` | 20246 | 14875 | -5371 |
| `provider_usage.output` | 166 | 174 | 8 |
| `provider_usage.reasoning` | 58 | 40 | -18 |
| `duration_ms` | 25099 | 25666 | 567 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 20674 | 20674 |
| Emitted source bytes | 20674 | 1339 |

Invariant gate: **true**.
Source output reduction: **93.52%**.
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
