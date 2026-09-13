# Tracepress Shadow Pilot 001 N10

Experiment: `shadow-pilot-001` · Status: **completed**

This report measures shadow candidate reduction only. It does not claim provider token, cost, cache, or quality impact.

## Integrity

- Forwarding mutations: 0
- Shadow drops: 15
- Recovery failures: 0
- Determinism failures: 0
- Unknown transformed: 0

## Candidate comparison

| Candidate | Applicable | Byte reduction | Est. token reduction | Recovery | P95 |
|---|---:|---:|---:|---:|---:|
| `json.minify` | 0.00% | — | — | 100.00% | 5201.7 us |
| `json.noop` | 0.00% | — | — | 100.00% | 3218.7 us |
| `json.repeated_subtree` | 0.00% | — | — | — | 5202.6 us |
| `json.tabular` | 43.77% | 0.12% | 0.18% | 100.00% | 5676.4 us |

## Collection notes

- The first valid session saturated the bounded shadow path and the experiment recorded 15 aggregate shadow drops.
- One code-review attempt was interrupted after its nested review workflow stalled and is excluded from N10.
- One CLI argument-error attempt created no provider requests and is excluded from N10.
- Aggregate experiment counters include excluded attempts because Phase 4.1 does not persist per-session shadow-drop attribution.

## Empirical gates

- `shadow_pilot_n10`: **completed_degraded**
- `compaction_calibration`: **completed_controlled_cohort**
- `aa_infrastructure`: **completed_smoke**
- `host_rss_cpu_comparison`: **pending**
- `naturalistic_workload_distributions`: **completed**

## Recommendation

No active candidate selected. Phase 4.2 remains blocked.
