# Tracepress Shadow Pilot 002 Directed

Experiment: `shadow-pilot-002` · Status: **completed**

This report measures shadow candidate reduction only. It does not claim provider token, cost, cache, or quality impact.

## Integrity

- Forwarding mutations: 0
- Shadow drops: 0
- Recovery failures: 0
- Determinism failures: 0
- Unknown transformed: 0

## Candidate comparison

| Candidate | Applicable | Byte reduction | Est. token reduction | Recovery | P95 |
|---|---:|---:|---:|---:|---:|
| `json.minify` | 0.00% | — | — | 100.00% | 1902.5 us |
| `json.noop` | 0.00% | — | — | 100.00% | 618.8 us |
| `json.repeated_subtree` | 0.00% | — | — | — | 2664.8 us |
| `json.tabular` | 100.00% | 41.04% | 40.56% | 100.00% | 2775.0 us |
| `text.noop` | 0.00% | — | — | 100.00% | 371.4 us |
| `text.repeated_line` | 100.00% | 47.06% | — | 100.00% | 309.8 us |
| `text.repeated_run` | 0.00% | — | — | 100.00% | 459.3 us |

## Collection notes

- Ten isolated sessions used a deterministic local upstream through the real proxy path.
- Six sessions carried homogeneous array<object> ToolResult JSON and four carried repetitive ToolResult PlainText.
- The cohort is directed synthetic characterization; naturalistic workload distribution evidence is inherited from Shadow Pilot 001.
- Gate statuses combine this clean directed run with the Pilot 001 naturalistic cohort, compaction calibration, A/A, and resource artifacts.
- The recommendation is for one future Phase 4.2 evaluation candidate only, never an assertion of provider token or quality impact.

## Empirical gates

- `shadow_pilot_n10`: **passed**
- `compaction_calibration`: **passed**
- `aa_infrastructure`: **passed**
- `host_rss_cpu_comparison`: **passed**
- `naturalistic_workload_distributions`: **passed**

## Recommendation

Recommended active candidate: `json.tabular`.
