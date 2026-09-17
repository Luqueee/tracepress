# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckV2Control`. Treatment: `ExplicitCheckV2Active`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 48395 | 47882 | -513 |
| `provider_usage.input_cached` | 33280 | 33280 | 0 |
| `provider_usage.input_uncached` | 15115 | 14602 | -513 |
| `provider_usage.output` | 180 | 145 | -35 |
| `provider_usage.reasoning` | 52 | 43 | -9 |
| `duration_ms` | 11501 | 10827 | -674 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 1507 | 1507 |
| Emitted source bytes | 1507 | 230 |

Invariant gate: **true**.
Source output reduction: **84.74%**.
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
