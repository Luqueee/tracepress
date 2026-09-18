# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitRgControl`. Treatment: `ExplicitRgShadow`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 56179 | 49039 | -7140 |
| `provider_usage.input_cached` | 33280 | 33280 | 0 |
| `provider_usage.input_uncached` | 22899 | 15759 | -7140 |
| `provider_usage.output` | 145 | 131 | -14 |
| `provider_usage.reasoning` | 44 | 50 | 6 |
| `duration_ms` | 10958 | 10754 | -204 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 28878 | 28878 |
| Emitted source bytes | 28878 | 28878 |

Invariant gate: **true**.
Source output reduction: **31.19%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
