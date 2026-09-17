# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitControl`. Treatment: `ExplicitShadow`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 20 | 0.0 |
| `provider_usage.input_total` | 407457 | 408293 | -22.0 |
| `provider_usage.input_cached` | 248832 | 265216 | 0.0 |
| `provider_usage.input_uncached` | 158625 | 143077 | -32.0 |
| `provider_usage.output` | 1724 | 1649 | -3.0 |
| `provider_usage.reasoning` | 410 | 424 | 4.5 |
| `duration_ms` | 190869 | 178229 | -1755.5 |

Invariant gate: **true**.
Source candidate reduction: **93.99%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
