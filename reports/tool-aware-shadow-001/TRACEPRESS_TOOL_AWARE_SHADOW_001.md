# TRACEPRESS_TOOL_AWARE_SHADOW_001

Experiment: `phase-4-4-shell-shadow-001`  
Status: **completed**  
Privacy: **aggregated metadata-only**

## Cohort

- Sessions: **10/10 successful read-only sessions**
- Provider requests: **21**
- Context analyses: **21/21 complete**
- ToolResult family: `shell_generic / json`
- ToolResult exposure: **18,257 estimated tokens**
- ToolResult PlainText: **0 blocks**

## Shadow integrity

- Shadow jobs admitted/processed/dropped: **21 / 21 / 0**
- Candidate evaluations attempted/completed/dropped: **117 / 117 / 0**
- Forwarding mutations: **0**
- Recovery failures: **0**
- Determinism failures: **0**
- Unknown transformed: **0**

## Shell reducer

| Reducer | Eligible | Applicable | Addressable share | Recovery | Determinism |
|---|---:|---:|---:|---:|---:|
| `shell.diagnostic_projection` | 9 | 0 | 0.00% | — | — |

The reducer intentionally requires a bounded shell-shaped object with an explicit execution status
and multiline `stdout`/`output`. The observed nested JSON did not satisfy that profile. This is a
fail-closed no-applicability result, not a reason to broaden the reducer heuristically.

## Decision

No reducer passes the Phase 4.4 shadow opportunity gate. Active Control/Treatment assignment is
blocked. The quality harness is ready for a future family with a material, semantically explicit
projection; no provider-token, cache, cost, or quality claim is made here.
