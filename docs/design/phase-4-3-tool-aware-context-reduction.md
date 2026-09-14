# Phase 4.3 — Tool-Aware Context Reduction

## Initial implementation boundary

Phase 4.2.3 is closed as `CLOSED — INSUFFICIENT MATERIAL OPPORTUNITY`. The
deterministic/reversible/provider-compatible candidates remain available as controls, but no
active candidate was selected.

Phase 4.3 therefore starts with a shadow-only, quality-gated reducer boundary. The current
implementation does not rewrite provider requests and cannot affect forwarding latency or bytes.

## Safety contract

Only blocks satisfying all of the following are eligible:

```text
origin          = ToolGenerated
kind            = ToolResult
detected kind   = Json
```

`Unknown`, instructions, human-authored content, agent messages, tool schemas, reasoning, and
provider-managed state remain `NeverTransform`.

The reducer is fail-closed. Invalid JSON, input/output/memory/work/time limit violations, and
non-improving views produce metadata-only terminal states and no visible candidate. Candidate and
original bytes are transient evaluation material; persistence receives fingerprints and bounded
metrics only.

## First reducer

`json.empty_noise_fields` is the first Phase 4.3 reducer. It removes only object fields whose
values are structurally empty (`null`, empty string, empty array, or empty object), recursively.
It is intentionally classified as `KeepFullWithCandidate` and remains shadow-only until objective
quality evaluation exists. It does not claim that an empty value is semantically disposable.

The reducer records:

```text
input/output bytes
estimated token counts when available
omitted field count
processing time
original/visible fingerprints
first modified offset and preserved prefix bytes
recovery availability and verification
determinism result
```

Recovery is local and bounded. The provider never receives a recovery id or a reducer-specific
decoder in this phase.

The second reducer, `json.repeated_value_elision`, applies only to homogeneous arrays of objects
with exact repeated primitive values. It emits ordinary JSON shaped as `common` plus `rows`, and
rejects duplicate keys, heterogeneous rows, unsupported nested common values, and every bounded
resource failure. It is also shadow-only.

## Shadow integration

The reducers run after context analysis in the existing bounded shadow worker. Each is accounted
as one candidate evaluation per eligible block, shares the shadow work budget, and is persisted
through the existing metadata-only candidate tables under its explicit reducer id. The manifest
identifies each reducer version so later cohorts remain comparable.

Forwarding remains byte-exact. Active request rewriting, recovery tools, lossy A/B assignment,
quality claims, and economic claims are not implemented by this milestone.

## Next gate

Before any active experiment, add metadata-only ToolResult family characterization and an
objective quality task/evaluator. A reducer may advance only after a naturalistic shadow cohort
shows material addressable exposure, bounded resources, deterministic replay, and verified local
recovery. Active assignment must be session-level with control/treatment persistence and fail-open
to the original ToolResult.

## Current milestone verification

The controlled shadow smoke persists `json.empty_noise_fields` as a metadata-only candidate and
asserts that the upstream request remains byte-exact. The synthetic Observatory fixture exposes the
reducer in the Compression Lab with an explicit `SHADOW ONLY` label; fixture values are not
efficacy evidence.

The current workspace validation is:

```text
cargo test --workspace       557 passed, 1 ignored
cargo clippy --workspace     clean
cargo check --workspace      clean
```

No naturalistic lossy pilot, objective quality cohort, active assignment, provider usage claim, or
economic claim has been made yet.

The shared compression crate now also exposes metadata-only quality contracts for
`ExitCode`, `TestSuite`, `FileChange`, and `KnownAnswer` evaluators. They describe task
outcomes without carrying prompts, commands, paths, source, or tool-result content; execution
remains a later bounded CLI concern.

## Shadow Pilot 001

The first naturalistic lossy shadow cohort completed with 10/10 successful read-only sessions:

```text
28/28 provider requests analyzed
28/28 shadow jobs processed
305/305 candidate evaluations completed
0 drops
0 forwarding mutations
0 recovery failures
0 determinism failures
0 Unknown transformations
```

`json.empty_noise_fields` had 32 eligible ToolResult JSON blocks and 0 applicable blocks. This
is a negative result for this reducer on this cohort, not a universal impossibility claim. The
metadata-only report is:

```text
reports/tool-aware-reduction-001/TRACEPRESS_TOOL_AWARE_SHADOW_PILOT_001.{json,md}
```

## Shadow Pilot 002

The second naturalistic cohort added `json.repeated_value_elision`, a human-readable reducer for
homogeneous arrays with exact constant primitive fields. It completed with full shadow integrity:

```text
10/10 read-only sessions successful
23/23 provider requests analyzed
23/23 shadow jobs processed
149/149 candidate evaluations completed
0 drops
0 forwarding mutations
0 recovery failures
0 determinism failures
0 Unknown transformations
```

The cohort contained 17 metadata-classified `array_object` JSON blocks. Both tool-aware reducers
had 15 eligible blocks and 0 applicable blocks; the existing human-readable controls were also
not applicable. This is a workload-specific negative result and does not justify a universal
claim. The metadata-only report is:

```text
reports/tool-aware-reduction-001/TRACEPRESS_TOOL_AWARE_SHADOW_PILOT_002.{json,md}
```

The active treatment gate remains blocked: no provider-token, cache, cost, or quality claim is
made, and no request rewriting is enabled.

## Shadow Pilot 003 and materiality gate

The final characterization cohort completed with 10/10 successful read-only sessions and full
shadow integrity:

```text
24/24 provider requests analyzed
24/24 shadow jobs processed
187/187 candidate evaluations completed
0 drops
0 forwarding mutations
0 recovery failures
0 determinism failures
0 Unknown transformations
```

The family matrix resolved the earlier PlainText ambiguity: this cohort contained 21
`ToolGenerated / ToolResult / Json` blocks (82,612 estimated tokens) and zero
`ToolGenerated / ToolResult / PlainText` blocks. The large PlainText population was in excluded
human/agent-authored families.

`json.repeated_value_elision` applied to 2/20 eligible JSON blocks, covering 2.49% of eligible
tokens, with 0.67% byte reduction and 0.68% estimated-token reduction. Recovery and determinism
were both 100%, but the result is below the materiality gate; no active candidate is selected.
`json.tabular` reached the same two blocks only as an opaque shadow upper bound. The report is:

```text
reports/tool-aware-reduction-001/TRACEPRESS_TOOL_AWARE_SHADOW_PILOT_003.{json,md}
```

Phase 4.3 remains shadow-only. No provider usage, cache, cost, or quality claim is made, and
active rewriting stays blocked pending a larger, tool-aware reduction design.
