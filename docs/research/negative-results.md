# Tracepress negative-results registry

This registry records measured candidates that are not active policies. Entries are metadata-only
and intentionally do not include prompts, paths, tool arguments, raw results, or candidate bytes.

| Candidate | Scope | Result | Decision |
|---|---|---|---|
| `json.minify` | ToolResult JSON | No material real improvement | Control only |
| `json.readable_table` | Homogeneous ToolResult JSON | No material applicability in tested cohorts | Closed for active use |
| `json.compact_records` | Homogeneous ToolResult JSON | No material applicability in tested cohorts | Closed for active use |
| `json.key_elision` | Homogeneous ToolResult JSON | No material applicability in tested cohorts | Closed for active use |
| `json.empty_noise_fields` | ToolResult JSON | 0 applicable blocks in the naturalistic reducers cohorts | Shadow only |
| `json.repeated_value_elision` | ToolResult JSON | 2.49% addressable share and 0.68% estimated reduction in the final pilot | Below materiality gate |
| `json.tabular` | ToolResult JSON | Small structural upper bound; opaque custom representation | Shadow-only upper bound |
| `search.result_projection` over provider-native envelope | Controlled public Search workload | Native envelopes were below the material reduction threshold | Closed for current candidates |
| `toolresult.lifetime` historical eviction | Public multi-request shadow cohort | 3.04% historical exposure; best net total-context reduction 2.76%, below Phase 4.6 gates | Closed without recovery or active eviction |
| `source.passthrough` Codex hook wrapper | Fully isolated `cargo test` N=10 A/A | Hook/proxy instrumentation was complete, but passthrough changed provider input materially (433,889 to 308,562) with no reducer enabled | Reject causal interpretation; require byte-faithful control evidence before Shadow |
| `source.path_shim` | Codex PATH interception smoke | Agent exited before provider, hook, or source execution; no sandbox bypass attempted | Closed for this runtime |

The evidence is workload-scoped. It closes generic deterministic/provider-readable expansion for
the current cohorts; it does not claim that every future tool-family policy is impossible.

Phase 4.4 therefore selects a family from metadata-only exposure before implementing a reducer.
