# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckControl`. Treatment: `ExplicitCheckShadow`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 48356 | 48312 | -44 |
| `provider_usage.input_cached` | 33280 | 19968 | -13312 |
| `provider_usage.input_uncached` | 15076 | 28344 | 13268 |
| `provider_usage.output` | 123 | 140 | 17 |
| `provider_usage.reasoning` | 48 | 52 | 4 |
| `duration_ms` | 10962 | 10843 | -119 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 1507 | 1507 |
| Emitted source bytes | 1507 | 1507 |

Invariant gate: **true**.
Source output reduction: **94.76%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
