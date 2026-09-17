# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckV2Control`. Treatment: `ExplicitCheckV2Active`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 21 | 0.0 |
| `provider_usage.input_total` | 483827 | 502986 | -491.5 |
| `provider_usage.input_cached` | 420864 | 444160 | 0.0 |
| `provider_usage.input_uncached` | 62963 | 58826 | -476.5 |
| `provider_usage.output` | 1565 | 1787 | 20.0 |
| `provider_usage.reasoning` | 509 | 662 | 15.5 |
| `duration_ms` | 111656 | 127093 | 259.5 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 15070 | 15070 |
| Emitted source bytes | 15070 | 2300 |

Invariant gate: **true**.
Source output reduction: **84.74%**.
Active gate: **true**.
Positive gate: **true**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
