# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitGitStatusControl`. Treatment: `ExplicitGitStatusShadow`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 481668 | 481610 | -14.0 |
| `provider_usage.input_cached` | 377344 | 366080 | 0.0 |
| `provider_usage.input_uncached` | 104324 | 115530 | 24.0 |
| `provider_usage.output` | 1358 | 1286 | -8.0 |
| `provider_usage.reasoning` | 533 | 482 | -9.5 |
| `duration_ms` | 114751 | 123298 | 369.0 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 4530 | 4530 |
| Emitted source bytes | 4530 | 4530 |

Invariant gate: **true**.
Source output reduction: **55.19%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
