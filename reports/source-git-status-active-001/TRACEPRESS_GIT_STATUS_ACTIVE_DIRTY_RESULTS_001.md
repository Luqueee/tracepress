# Tracepress explicit source-tool experiment

Decision: **REJECT**.

Control: `ExplicitGitStatusActiveControl`. Treatment: `ExplicitGitStatusActive`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 482120 | 481936 | -12.5 |
| `provider_usage.input_cached` | 425984 | 423936 | 0.0 |
| `provider_usage.input_uncached` | 56136 | 58000 | -9.5 |
| `provider_usage.output` | 1242 | 1334 | 11.0 |
| `provider_usage.reasoning` | 402 | 472 | 7.5 |
| `duration_ms` | 108167 | 120625 | 12.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 4530 | 4530 |
| Emitted source bytes | 4530 | 3450 |

Invariant gate: **true**.
Source output reduction: **23.84%**.
Active gate: **true**.
Positive gate: **false**.
Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
