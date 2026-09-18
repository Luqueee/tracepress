# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitRgControl`. Treatment: `ExplicitRgShadow`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 532697 | 538511 | -17.5 |
| `provider_usage.input_cached` | 358400 | 319488 | -2048.0 |
| `provider_usage.input_uncached` | 174297 | 219023 | 4974.0 |
| `provider_usage.output` | 1403 | 1319 | -15.0 |
| `provider_usage.reasoning` | 555 | 431 | -17.5 |
| `duration_ms` | 120585 | 108408 | -7.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 288780 | 288780 |
| Emitted source bytes | 288780 | 288780 |

Invariant gate: **true**.
Source output reduction: **31.19%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
