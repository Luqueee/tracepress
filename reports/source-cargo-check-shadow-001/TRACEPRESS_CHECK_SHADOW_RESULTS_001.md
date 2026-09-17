# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckControl`. Treatment: `ExplicitCheckShadow`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 482764 | 483229 | -8.0 |
| `provider_usage.input_cached` | 364544 | 387072 | 0.0 |
| `provider_usage.input_uncached` | 118220 | 96157 | -43.5 |
| `provider_usage.output` | 1588 | 1595 | 0.5 |
| `provider_usage.reasoning` | 524 | 463 | -6.0 |
| `duration_ms` | 116542 | 115198 | -151.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 15070 | 15070 |
| Emitted source bytes | 15070 | 15070 |

Invariant gate: **true**.
Source output reduction: **94.76%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
