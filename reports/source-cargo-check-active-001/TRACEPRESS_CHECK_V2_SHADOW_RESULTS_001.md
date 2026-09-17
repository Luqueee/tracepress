# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitCheckV2Control`. Treatment: `ExplicitCheckV2Shadow`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 22 | 21 | 0.0 |
| `provider_usage.input_total` | 532227 | 507347 | -70.5 |
| `provider_usage.input_cached` | 431616 | 434944 | 0.0 |
| `provider_usage.input_uncached` | 100611 | 72403 | -2095.0 |
| `provider_usage.output` | 1854 | 1499 | -14.5 |
| `provider_usage.reasoning` | 585 | 496 | -10.0 |
| `duration_ms` | 121253 | 127556 | 192.5 |

| Trajectory metric | Control | Treatment |
|---|---:|---:|
| Task success | 10/10 | 10/10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Raw source bytes | 15070 | 15070 |
| Emitted source bytes | 15070 | 15070 |

Invariant gate: **true**.
Source output reduction: **89.98%**.
Shadow gate: **true**.
Agent-visible output remained raw; provider deltas are not source-reduction savings.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
