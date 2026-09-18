# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitGitStatusControl`. Treatment: `ExplicitGitStatusShadow`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 48153 | 48166 | 13 |
| `provider_usage.input_cached` | 44544 | 29184 | -15360 |
| `provider_usage.input_uncached` | 3609 | 18982 | 15373 |
| `provider_usage.output` | 121 | 137 | 16 |
| `provider_usage.reasoning` | 43 | 48 | 5 |
| `duration_ms` | 16987 | 11728 | -5259 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 453 | 453 |
| Emitted source bytes | 453 | 453 |

Invariant gate: **true**.
Source output reduction: **55.19%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
