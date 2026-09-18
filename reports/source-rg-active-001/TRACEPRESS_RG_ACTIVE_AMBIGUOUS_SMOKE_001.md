# Tracepress explicit source-tool experiment

Decision: **PASS**.

Control: `ExplicitRgActiveControl`. Treatment: `ExplicitRgActive`. Pairs: 1.

| Metric | Control total | Treatment total | Paired median delta |
|---|---:|---:|---:|
| `provider_requests` | 2 | 2 | 0 |
| `provider_usage.input_total` | 50646 | 60467 | 9821 |
| `provider_usage.input_cached` | 44544 | 31232 | -13312 |
| `provider_usage.input_uncached` | 6102 | 29235 | 23133 |
| `provider_usage.output` | 121 | 148 | 27 |
| `provider_usage.reasoning` | 45 | 44 | -1 |
| `duration_ms` | 10973 | 10850 | -123 |

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
Active gate: **true**.
Positive gate: **pending full cohort**.
Treatment fail-open preserved raw output; downstream deltas are safety A/A evidence.

No prompts, commands, paths, raw tool output, or provider bodies are present in this report.
