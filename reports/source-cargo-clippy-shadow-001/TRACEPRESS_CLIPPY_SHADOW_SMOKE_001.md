# Tracepress explicit source-tool experiment

Decision: **REJECT**.

Control: `ExplicitClippyControl`. Treatment: `ExplicitClippyShadow`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 50377 | 58727 | 8350 |
| `provider_usage.input_cached` | 44544 | 44544 | 0 |
| `provider_usage.input_uncached` | 5833 | 14183 | 8350 |
| `provider_usage.output` | 108 | 122 | 14 |
| `provider_usage.reasoning` | 32 | 48 | 16 |
| `duration_ms` | 10759 | 10783 | 24 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 136328 | 136328 |
| Emitted source bytes | 136328 | 136328 |

Invariant gate: **true**.
Source output reduction: **0.99%**.
Shadow gate: **false**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
