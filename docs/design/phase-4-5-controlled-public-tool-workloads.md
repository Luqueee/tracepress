# Phase 4.5 — Controlled Public Tool Workloads

## Scope and safety

Phase 4.5 keeps request forwarding byte-exact and shadow-only. Public repositories are cloned at
fixed commits under `/tmp/tracepress-public-workloads-001`; no source, command, working directory,
or ToolResult bytes are written to Tracepress. Kena is not used by this phase.

The shell family is refined in memory into:

```text
search | tests | build | lint | dependency | version_control | generic | unknown
```

Only the allowlisted family label can cross an aggregate report boundary. Conflicting command and
output signals fail closed to `unknown`.

## Canonical models and reducer

`tracepress-compression` now contains transient-only `SearchResultModel` and `TestResultModel`.
`SearchResultReducer` (`search.result_projection`) groups all parsed matches by file while
retaining line numbers and match text. It is semantically lossless for its canonical model,
deterministic, bounded, and locally recoverable byte-for-byte. It is not enabled for forwarding.

The shadow worker correlates a ToolResult's bounded call id with the matching ToolCall argument
span, classifies the command/output signals in memory, and evaluates the search reducer only for
the `search` family. It does not persist commands or arguments.

## Public cohort

`scripts/run_public_tool_workloads_001.py` runs 12 bounded tasks:

```text
6 Search tasks on BurntSushi/ripgrep
6 Tests tasks on a temporary synthetic overlay executed through pinned pytest source
```

The repositories are pinned to the exact commits in the experiment manifest. Captured output is
fed to the Rust shadow helper through stdin and is discarded after metric extraction. Reports are
metadata-only and contain no source, paths, commands, or output lines.

## Result

The report is:

```text
reports/public-tool-workloads-001/TRACEPRESS_PUBLIC_TOOL_WORKLOADS_001.{json,md}
```

Measured result:

```text
Search exposure:       68.94% of bounded cohort tokens
Search applicability:  6/6
Search reduction:      29.74% of total bounded cohort bytes
Canonical correctness: 100%
Recovery:              100%
Determinism:           100%

Tests exposure:        31.06%
Tests reducer:         characterization only

Shadow jobs:           12/12 processed
Shadow drops:           0
Forwarding mutations:   0
```

These are local representation measurements for a controlled public workload. They are not
provider-token savings, cache effects, cost claims, or agent-quality results.

## Decision

`search.result_projection` passes the Phase 4.5 shadow opportunity gate for this workload class
and was exercised only by a bounded infrastructure diagnostic. The treatment path reached the
proxy, but no provider-native Search span was evaluated, so no quality comparison is valid. A
future quality pilot must use the same pinned repositories and task IDs, with objective outcomes,
session-level assignment, fail-open behavior, and no Kena input.

## Quality-pilot preparation

The reproducible Search answer key and a planned Control/Treatment schedule are prepared by:

```text
scripts/prepare_public_search_quality_pilot_001.py
```

The metadata-only artifact is:

```text
reports/public-tool-workloads-001/TRACEPRESS_SEARCH_QUALITY_PILOT_PREPARATION_001.{json,md}
```

This preparation deliberately starts zero provider sessions and performs zero request rewrites.
It stores only aggregate match/file counts, public repository provenance, evaluator metadata, and
planned session assignments. The active quality comparison remains gated until the runner is reviewed
for fail-open behavior, recovery isolation, and provider usage instrumentation.

The reviewed runner is:

```text
scripts/run_public_search_quality_pilot_001.py
```

Its first bounded run completed with zero active Search rewrites: the treatment path was reached,
but no eligible Search span was evaluated. This is recorded as an infrastructure diagnostic, not
as a quality result; active quality comparison remains invalid until the provider-native ToolResult
shape is characterized and the reducer reaches a real evaluated span.

The diagnostic artifact is:

```text
reports/public-tool-workloads-001/TRACEPRESS_SEARCH_QUALITY_ACTIVE_PILOT_001.{json,md}
```

## Provider-native shape diagnostic

`scripts/characterize_public_provider_native_search_001.py` runs one additional public,
read-only session with active compression disabled. It records only aggregate Context Analysis
labels and sizes; prompts, commands, paths, source, ToolResult bytes, responses, and fingerprints
remain transient and are deleted with its `/tmp` state.

The diagnostic recorded two completed provider/context observations and one native ToolResult.
That ToolResult was classified as `tool_generated / tool_result / json` (386 bytes; 63 locally
estimated tokens), rather than `plain_text` or `search_results`. Consequently the active selector
correctly produced zero evaluated spans: the current reducer accepts raw ripgrep text, while the
provider-native result is a JSON envelope.

The metadata-only artifact is:

```text
reports/public-tool-workloads-001/TRACEPRESS_PUBLIC_PROVIDER_NATIVE_SEARCH_SHAPE_001.{json,md}
```

The adapter now preserves a valid envelope byte-for-byte except for one uniquely identified,
canonical Search-output string. It is deterministic, has exact in-memory recovery, rejects
ambiguous envelopes, and is covered by local canonical-model tests. Shadow scheduling evaluates
JSON ToolResults through that strict parser even when a provider's transient tool-name label does
not identify a shell surface; this replaces silent missingness with an explicit terminal state.

The follow-up metadata-only shadow run is:

```text
reports/public-tool-workloads-001/TRACEPRESS_PUBLIC_PROVIDER_NATIVE_SEARCH_ENVELOPE_SHADOW_001.{json,md}
```

It observed a native JSON ToolResult, but its decoded representation was only 123 bytes (63
locally estimated tokens). The Search reducer's conservative 256-byte policy therefore kept it
full and produced no Search candidate evaluation. This is neither a shadow drop nor a candidate
failure: the provider-native result is below the minimum size where an active rewrite could be
material. No further active quality session is justified for this task without evidence of larger
native Search ToolResults.
