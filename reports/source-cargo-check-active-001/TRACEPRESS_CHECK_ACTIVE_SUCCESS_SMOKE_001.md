# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckActiveControl`. Treatment: `ExplicitCheckActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 3 | 3 | 0 |
| `provider_usage.input_total` | 72615 | 72603 | -12 |
| `provider_usage.input_cached` | 43264 | 67840 | 24576 |
| `provider_usage.input_uncached` | 29351 | 4763 | -24588 |
| `provider_usage.output` | 270 | 334 | 64 |
| `provider_usage.reasoning` | 80 | 73 | -7 |
| `duration_ms` | 17620 | 16805 | -815 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 2 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 1 |
| Raw source bytes | 1507 | 1507 |
| Emitted source bytes | 1507 | 158 |

Invariant gate: **true**.
Source output reduction: **89.52%**.
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
