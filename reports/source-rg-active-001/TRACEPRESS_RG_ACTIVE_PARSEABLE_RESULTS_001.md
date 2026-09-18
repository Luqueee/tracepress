# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitRgActiveControl`. Treatment: `ExplicitRgActive`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 521550 | 521895 | 19.0 |
| `provider_usage.input_cached` | 386048 | 418816 | 0.0 |
| `provider_usage.input_uncached` | 135502 | 103079 | -8.5 |
| `provider_usage.output` | 1406 | 1545 | 12.5 |
| `provider_usage.reasoning` | 470 | 488 | 3.0 |
| `duration_ms` | 116979 | 114494 | -91.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 288780 | 288780 |
| Emitted source bytes | 288780 | 200270 |

Invariant gate: **true**.
Source output reduction: **30.65%**.
Active gate: **true**.
Positive gate: **true**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
