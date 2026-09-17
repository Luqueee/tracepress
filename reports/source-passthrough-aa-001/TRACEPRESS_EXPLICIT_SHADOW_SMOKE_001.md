# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitControl`. Treatment: `ExplicitShadow`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 41065 | 41060 | -5 |
| `provider_usage.input_cached` | 19968 | 28160 | 8192 |
| `provider_usage.input_uncached` | 21097 | 12900 | -8197 |
| `provider_usage.output` | 175 | 153 | -22 |
| `provider_usage.reasoning` | 42 | 24 | -18 |
| `duration_ms` | 17754 | 18157 | 403 |

Invariant gate: **true**.
Source candidate reduction: **93.99%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
