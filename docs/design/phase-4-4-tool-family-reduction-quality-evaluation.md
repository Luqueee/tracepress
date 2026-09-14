# Phase 4.4 — Tool-Family Reduction & Quality Evaluation

## Baseline

Phase 4.3 is frozen at:

```text
commit: 03c886b2249a019827a8c5a4cb54c29890f1454a
tag:    phase-4.3-shadow-complete
```

Generic deterministic/provider-readable JSON reduction is closed as insufficiently material for
the tested workloads. Kena is characterization-only and is not an active reduction or quality
target for this phase.

## Family taxonomy

Tool names are classified in memory into an allowlisted metadata label only:

```text
search
tests
build
lint
dependency
version_control
filesystem
shell_generic
structured_data
unknown
```

Raw tool names, commands, arguments, paths, prompts, and outputs are never included in reports.
Ambiguous values are classified as `unknown`.

## Characterization gate

The first milestone is `TOOL_FAMILY_CHARACTERIZATION_001`. It ranks ToolResult exposure by:

```text
family
detected kind
block count
estimated tokens
raw bytes
P50/P90/P95/P99 block size
exact repetition and unique fingerprint counts
```

The report keeps JSON, PlainText, and other detected kinds separate. No reducer is selected until
one family demonstrates material exposure and sufficiently large blocks.

## Quality harness

The shared compression crate now contains metadata-only contracts for:

```text
ToolFamily
QualityTask
QualityAssignment
QualityExperimentArm
QualitySessionMetrics
QualityOutcome
```

Quality truth is objective (`Build`, `ExitCode`, `TestSuite`, `KnownAnswer`, or `FileChange`).
Prompts, paths, commands, source, and tool-result content are outside the DTO boundary. NULL
provider or evaluator values remain NULL and are never converted to zero.

Active assignment is not implemented yet. When a family passes the shadow gate, the first pilot
will use one reducer, one family, session-level Control/Treatment assignment, fail-open recovery,
and paired objective tasks on public or synthetic repositories only.

## Safety gates

Before any active experiment:

```text
0 shadow drops
0 reducer failures
0 forwarding mutations
0 recovery-isolation failures
0 cross-session recovery leaks
```

The lossy opportunity gate is approximately 5% addressable context and 5% effective total-context
reduction. It is an investigation filter, not a claim of provider savings.

Kena remains outside active experiments until the quality harness, recovery isolation, and privacy
review have independently passed on public/controlled workloads.

## Characterization 001 result

The first controlled N=10 cohort completed with:

```text
20/20 provider requests analyzed
20/20 shadow jobs processed
90/90 candidate evaluations completed
0 drops
0 forwarding mutations
0 Unknown transformations
```

`shell_generic` is the only observed ToolResult family and accounts for 27,566 estimated tokens
(100% of ToolResult exposure in this cohort). Its blocks are nested JSON with P50 1,101 bytes and
P95/P99 18,503 bytes. Exact repetition was not observed. ToolResult PlainText remained absent.

The bounded extractor now correlates a result's `call_id` with the matching bounded tool name so
family classification does not fall back to `unknown`; it does not retain arguments or output
content. The metadata-only artifact is:

```text
reports/tool-family-characterization-001/TRACEPRESS_TOOL_FAMILY_CHARACTERIZATION_001.{json,md}
```

This selects `shell_generic` for deeper shadow characterization, but no reducer is active yet.
The nested shape alone is insufficient evidence to omit a semantic field. The next step is one
shell-specific shadow reducer plus objective public/controlled quality tasks; Control/Treatment
assignment remains blocked.

## Shell shadow result

The selected family was evaluated with exactly one reducer,
`shell.diagnostic_projection`. It requires an explicit execution status and multiline `stdout` or
`output`, and emits a bounded head/tail view with a recovery marker. The naturalistic shadow
cohort completed with:

```text
10/10 sessions successful
21/21 provider requests analyzed
21/21 shadow jobs processed
117/117 candidate evaluations completed
0 drops
0 forwarding mutations
0 recovery failures
0 determinism failures
0 Unknown transformations
```

The reducer had 9 eligible blocks and 0 applicable blocks. The observed nested JSON did not match
the explicit shell-output profile, so the reducer failed closed rather than broadening heuristics.
The metadata-only report is:

```text
reports/tool-aware-shadow-001/TRACEPRESS_TOOL_AWARE_SHADOW_001.{json,md}
```

No family-specific reducer currently passes the Phase 4.4 opportunity gate. The quality harness
is implemented and ready, but active Control/Treatment assignment remains blocked until a future
public/controlled workload provides a material, semantically explicit projection.
