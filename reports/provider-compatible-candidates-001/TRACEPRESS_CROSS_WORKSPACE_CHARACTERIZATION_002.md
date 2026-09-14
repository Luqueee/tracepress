# Tracepress Cross-Workspace Characterization 002

Metadata-only external-private cohort. No prompts, source, paths, raw tool results, payloads, or
candidate bodies are included.

## Integrity

- Sessions: **10**
- Provider requests: **88**
- Complete analyses: **88/88**
- Shadow jobs admitted/processed/dropped: **64 / 64 / 0**
- Candidate evaluations attempted/completed/dropped: **1638 / 1638 / 0**
- Forwarding mutations: **0**
- Recovery failures: **0**
- Determinism failures: **0**
- Unknown transformed: **0**

## Exposure

ToolResult JSON: **510 blocks**, **1,512,057 estimated tokens**.

ToolResult PlainText: **0 blocks**, **0 estimated tokens**.

## Candidate decision

Human-readable provider-compatible candidates had zero applicability. `json.tabular` applied to one
block only and remains an opaque shadow-only upper bound. `json.minify` remains a no-improvement
control. No active candidate was selected.

**Decision:** close the deterministic/reversible/provider-compatible path as insufficiently
material and proceed to quality-led Tool-Aware Context Reduction.

