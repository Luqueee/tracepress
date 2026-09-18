# Tracepress explicit source-tool experiment

Decision: **REJECT**.

Control: `ExplicitRgControl`. Treatment: `ExplicitRgShadow`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 50480 | 50484 | 4 |
| `provider_usage.input_cached` | 19968 | 33280 | 13312 |
| `provider_usage.input_uncached` | 30512 | 17204 | -13308 |
| `provider_usage.output` | 141 | 139 | -2 |
| `provider_usage.reasoning` | 49 | 43 | -6 |
| `duration_ms` | 10858 | 10764 | -94 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 1/1 | 1/1 |
| Tool calls | 1 | 1 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 228262 | 228262 |
| Emitted source bytes | 228262 | 228262 |

Invariant gate: **true**.
Source output reduction: **0.00%**.
Shadow gate: **false**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
