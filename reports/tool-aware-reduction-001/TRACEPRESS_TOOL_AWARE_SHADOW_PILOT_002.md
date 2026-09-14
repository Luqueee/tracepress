# TRACEPRESS_TOOL_AWARE_SHADOW_PILOT_002

Experiment: `tool-aware-shadow-pilot-002`  
Status: **completed**  
Privacy: **aggregated metadata-only**

## Cohort

- Sessions: **10/10 successful read-only sessions**
- Provider requests: **23**
- Context analyses: **23/23 complete**
- Unknown transformed: **0**

## Shadow integrity

- Forwarding mutations: **0**
- Shadow jobs admitted/processed/dropped: **23 / 23 / 0**
- Candidate evaluations attempted/completed/dropped: **149 / 149 / 0**
- Persisted candidate rows: **149**
- Recovery failures: **0**
- Determinism failures: **0**

## Tool-aware candidates

| Candidate | Eligible | Applicable | Addressable share | Decision |
|---|---:|---:|---:|---|
| `json.empty_noise_fields` | 15 | 0 | 0.00% | No applicable real blocks |
| `json.repeated_value_elision` | 15 | 0 | 0.00% | No applicable real blocks |

All evaluated JSON blocks were classified as `array_object` at metadata level. Neither reducer
found a material candidate in this naturalistic cohort. The result is specific to this workload;
it is not a universal impossibility claim.

The existing provider-readable controls (`json.readable_table`, `json.compact_records`, and
`json.key_elision`) also had zero applicable blocks. The new reducer was nevertheless evaluated
with zero candidate drops and zero forwarding impact.

## Decision and limits

The active treatment gate remains blocked. No provider-token, provider-cache, cost, or quality
claim is made. Lossy active rewriting is not enabled. This pilot validates bounded scheduling,
deterministic replay, fail-closed applicability, and metadata-only persistence for the next
reducer iteration.
