# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitA`. Treatment: `ExplicitB`. Pairs: 10.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 20 | 21 | 0.0 |
| `provider_usage.input_total` | 409711 | 435764 | -17.5 |
| `provider_usage.input_cached` | 273408 | 300800 | 0.0 |
| `provider_usage.input_uncached` | 136303 | 134964 | -17.5 |
| `provider_usage.output` | 1820 | 1766 | -9.5 |
| `provider_usage.reasoning` | 436 | 395 | -3.0 |
| `duration_ms` | 181607 | 202335 | -498.5 |

Invariant gate: **true**.
Provider and trajectory deltas characterize A/A noise between identical arms.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
