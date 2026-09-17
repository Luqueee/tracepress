# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitActiveControl`. Treatment: `ExplicitActive`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 21 | 0.0 |
| `provider_usage.input_total` | 532117 | 505601 | -5384.5 |
| `provider_usage.input_cached` | 430080 | 440064 | 0.0 |
| `provider_usage.input_uncached` | 102037 | 65537 | -5361.5 |
| `provider_usage.output` | 1669 | 1782 | 9.0 |
| `provider_usage.reasoning` | 583 | 722 | 11.5 |
| `duration_ms` | 198031 | 186247 | -305.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 206740 | 206740 |
| Emitted source bytes | 206740 | 13390 |

Invariant gate: **true**.
Source output reduction: **93.52%**.
Active gate: **true**.
Positive gate: **true**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
