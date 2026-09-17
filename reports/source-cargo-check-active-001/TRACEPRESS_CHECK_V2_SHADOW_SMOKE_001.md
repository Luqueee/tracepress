# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckV2Control`. Treatment: `ExplicitCheckV2Shadow`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 48379 | 48266 | -113 |
| `provider_usage.input_cached` | 33280 | 33280 | 0 |
| `provider_usage.input_uncached` | 15099 | 14986 | -113 |
| `provider_usage.output` | 141 | 123 | -18 |
| `provider_usage.reasoning` | 35 | 57 | 22 |
| `duration_ms` | 10789 | 10755 | -34 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 1507 | 1507 |
| Emitted source bytes | 1507 | 1507 |

Invariant gate: **true**.
Source output reduction: **89.98%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
