# TRACEPRESS_TOOL_FAMILY_CHARACTERIZATION_001

Experiment: `phase-4-4-characterization-003`  
Status: **completed**  
Privacy: **aggregated metadata-only**

## Cohort

- Sessions: **10/10 successful read-only sessions**
- Provider requests: **20**
- Context analyses: **20/20 complete**
- Unknown transformed: **0**
- Shadow jobs admitted/processed/dropped: **20 / 20 / 0**
- Candidate evaluations attempted/completed/dropped: **90 / 90 / 0**
- Forwarding mutations: **0**

## ToolResult exposure ranking

| Family | Detected | Blocks | Estimated tokens | Exposure | P50 bytes | P95 bytes | P99 bytes | Exact repeated blocks |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `shell_generic` | `json` | 10 | 27,566 | 100.00% | 1,101 | 18,503 | 18,503 | 0 |

All observed ToolResults in this controlled cohort were classified as `shell_generic` using an
allowlisted bounded tool-name label. No raw tool names, commands, arguments, paths, prompts, or
outputs are included in this report.

## Structural characterization

```text
JSON shape: nested_array_or_object
ToolResult PlainText: 0 blocks
Exact repeated blocks: 0
Unique exact fingerprints: 10
```

Existing generic provider-readable candidates had zero applicable blocks in this cohort. This
report selects `shell_generic` as the first family for deeper characterization, but does not yet
select an active reducer: the observed nested shape is not sufficient evidence for safely omitting
any semantic field.

## Decision

Proceed with one shadow-only shell-specific reducer after adding a metadata-only schema profile and
objective quality tasks on public/controlled repositories. Active Control/Treatment assignment
remains blocked.
