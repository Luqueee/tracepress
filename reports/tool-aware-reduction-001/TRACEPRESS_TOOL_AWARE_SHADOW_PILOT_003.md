# TRACEPRESS_TOOL_AWARE_SHADOW_PILOT_003

Experiment: `tool-aware-shadow-pilot-003`  
Status: **completed**  
Privacy: **aggregated metadata-only**

## Cohort and family characterization

- Sessions: **10/10 successful read-only sessions**
- Provider requests: **24**
- Context analyses: **24/24 complete**
- Unknown transformed: **0**
- `ToolGenerated / ToolResult / Json`: **21 blocks**, **82,612 estimated tokens**
- `ToolGenerated / ToolResult / PlainText`: **0 blocks**
- PlainText observed in the cohort belonged to excluded human/agent-authored families.

## Shadow integrity

- Forwarding mutations: **0**
- Shadow jobs admitted/processed/dropped: **24 / 24 / 0**
- Candidate evaluations attempted/completed/dropped: **187 / 187 / 0**
- Recovery failures: **0**
- Determinism failures: **0**

## Candidate results

| Candidate | Eligible | Applicable | Addressable token share | Reduction |
|---|---:|---:|---:|---:|
| `json.empty_noise_fields` | 20 | 0 | 0.00% | — |
| `json.repeated_value_elision` | 20 | 2 | 2.49% | 0.67% bytes / 0.68% estimated tokens |
| `json.tabular` (opaque upper bound) | 21 | 2 | 2.49% | 0.57% bytes |

`json.repeated_value_elision` recovered both applicable candidates and was deterministic in all
repeated evaluations. The provider-readable candidate remains below the investigation gate of
approximately 5% addressable context and 2–3% effective context reduction. `json.tabular` is
retained only as a shadow upper bound because it is not provider-readable.

## Decision

This is a small, workload-specific opportunity, not a viable active candidate. Active request
rewriting and A/B assignment remain blocked. No provider-token, provider-cache, cost, or quality
claim is made.
