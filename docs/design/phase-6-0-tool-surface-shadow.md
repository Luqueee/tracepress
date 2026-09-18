# Phase 6.0 tool surface Shadow

Phase 6.0 begins the next research direction after source-output policy consolidation. It measures
the tool-definition surface already present in captured provider requests. It does not remove,
rewrite, reorder, or dynamically select tools.

## Question

Before building a tool-selection policy, Tracepress must establish whether observed sessions expose
a material schema surface and whether declared tools are actually used. The first slice therefore
answers only:

- how many tool definitions are exposed across the latest request snapshots;
- how many bytes and locally estimated tokens those schemas occupy;
- how much locally estimated schema material is repeated;
- how many explicit tool-call blocks are observed;
- how many session-scoped definition identities have a matching call identity; and
- what provider-reported usage exists for the same analyzed request cohort.

Provider totals are adjacent evidence, not attributed savings. Shadow mode cannot establish that a
smaller tool surface would change input, cache behavior, agent trajectory, or task quality.

## Existing metadata only

This phase adds no migration and no provider/runtime mutation. The read-only projection reuses:

```text
context_analysis_metrics.tool_count
context_analysis_metrics.schema_bytes
context_analysis_metrics.estimated_schema_tokens
context_analysis_metrics.repeated_schema_tokens
context_block_occurrences.kind
context_block_occurrences.tool_name
context_block_occurrences.tool_name_truncated
context_block_occurrences.tool_name_hash
provider_usage
```

Only the latest analysis version for each provider request contributes to the projection. Schema
aggregate coverage is reported explicitly, and every schema total/share uses only rows where the
complete schema metric set is present. Missing or disjoint partial aggregates remain unavailable
instead of being combined or converted to zero. Provider usage is restricted to the same analyzed
provider-request cohort.

## Privacy contract

`GET /api/v1/tool-surface` returns an explicit aggregate DTO. It never returns:

```text
tool names
tool identity material
schema bodies
tool arguments
tool outputs
prompts
session identifiers
request identifiers
paths
```

Distinct definitions and uses are computed with a session-scoped ephemeral identity. An exact
bounded name identifies an untruncated value; a truncated value is identified by its stored
full-value hash. This matches the existing producer contract without a migration. The same tool
exposed in two sessions counts as two session-scoped observations. Names, hashes, synthesized
identities, and session identifiers never cross the API boundary.

`unused_tools_lower_bound` is available only when at least one definition is present and every
observed definition and call block has a complete exact-name or truncated-hash identity. Partial
identity coverage, any partial/malformed/uncorrelated latest snapshot, or any provider request in
an observed session without a latest snapshot returns `null`; it does not manufacture a precise
unused count.

## Observatory contract

The Tool Surface page separates three groups:

1. schema exposure from local context analysis;
2. explicit tool usage from session-scoped identities; and
3. provider-reported usage for the same analyzed provider-request cohort.

The page is labeled `SHADOW ONLY` and states that no provider effect is active. Local estimates and
provider-reported usage retain distinct provenance.

## Phase boundary

Phase 6.0 proves the metadata-only measurement surface and its privacy/read-only properties. It
does not run a provider cohort and cannot accept or reject tool selection. A subsequent controlled
phase must use pinned public tasks, measure task success and trajectory, and compare identical
session-level arms before any request mutation is considered.
