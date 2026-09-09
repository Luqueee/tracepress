# Tracepress — Phase 3: Shadow Context Analysis

## 0. Baseline

Phase 0/1 and Phase 2 are closed. Phase 3 starts from this exact state:

```text
commit: 2b5fd6fa672887f60486d94661e7bf7e4e644561
tag:    phase-2-complete
branch: main
tests:  249 passed, 0 failed, 1 ignored
```

Protected artefacts, byte-identical for the whole of Phase 3:

```text
crates/tracepress-storage/migrations/0001_initial.sql
  sha256 4a08ae8e8bfd52f0eb427ccaed02b11bb1d6a12676b11efd77356b7e66acc119

crates/tracepress-storage/migrations/0002_provider_observability.sql
  sha256 1654365bc55c1e00796df87c1061514d70581fd7873d770654128b7dd824b2ec

PLAN_PHASE_2.md
```

Real defects discovered during Phase 3 are fixed with new commits and new
migrations. History is never rewritten.

The authoritative implementation contract for this phase is
`docs/design/phase-3-shadow-context-analysis.md` (task T1). This file is the
plan of record: scope, invariants, task breakdown, and Definition of Done.

## 1. Objective

Answer, without modifying any request:

> What exactly is the explicit context Tracepress can observe made of, how
> much does each part occupy, how much of it repeats, and which parts are
> candidates for future optimisation?

```text
Agent
  │ original request
  ▼
Proxy ───────────────────────────────► OpenAI
  │
  │ bounded low-copy tap
  ▼
Shadow Context Analyzer
  ├── structural index
  ├── context blocks
  ├── content classification
  ├── token estimates
  ├── repetition analysis
  ├── visibility analysis
  ├── stability analysis
  └── opportunity signals
        ▼
   tracepressd ──► SQLite
```

Invariant: Phase 3 ON and Phase 3 OFF send exactly the same bytes to the
provider.

Phase 3 OBSERVES, CLASSIFIES, MEASURES, CORRELATES. It does NOT TRANSFORM,
COMPRESS, or REWRITE.

Priority order for every decision in this phase:

```text
visibility correctness
> dataset integrity
> causal correlation
> measurement quality
> privacy
> resource bounds
> performance
> feature coverage
```

A future optimizer learns from this data. Biased data, unmarked
incompleteness, or semantically wrong data teaches a wrong policy.

## 2. Questions Phase 3 must answer

For any observed request: how many explicit blocks; bytes per block;
estimated tokens per block; the share of instructions, system/developer,
user, assistant history, tool definitions, tool calls, tool results, and
files/images/references; which blocks repeat between requests; which are
new; the stable explicit prefix; what changed; which tool results and tool
schemas dominate; whether a tool result looks like JSON, logs, search
results, tests, code, diffs, or plain text; how much context is directly
visible; how much depends on provider-managed state; and how much of the
billed input is explainable by visible content.

Phase 3 never claims "we can save X tokens". It reports "X estimated tokens
belong to candidate content". `candidate tokens` is not
`tokens actually saveable`.

## 3. Mandatory change 1 — correlation degradation is observable

Phase 2's bounds (64 in-flight forwards, 256 retired identities) stay
bounded and keep degrading gracefully, but the degradation must stop being
invisible. Add counters:

```text
correlation_degraded_total
correlation_degraded_inflight_limit
correlation_degraded_retired_limit
correlation_missing_total
```

and the event `context.correlation.degraded` carrying reason, session id and
timestamp — never content. A request that cannot be fully correlated is
recorded as `correlation_status = degraded`; complete causality is never
faked.

## 4. Mandatory change 2 — the IPC frame stays 32 KiB

The 32 KiB IPC body bound is NOT raised to carry prompts or tool outputs.
Phase 3 sends ids, hashes, spans, features, and metrics over IPC — never raw
prompts, raw tool results, or large JSON. Raw content stays under the
existing capture and persistence rules. Semantic records are compact.

## 5–11. Canonical domain

`ContextSnapshot { id, session_id, provider_request_id,
inference_operation_id, analysis_version, status, visibility,
request_content_hash, explicit_block_count, analyzed_bytes, skipped_bytes,
started_at_us, completed_at_us }` with a new `ContextSnapshotId` (UUIDv7).

`ContextAnalysisStatus { Complete, Partial, ResourceLimit, Malformed,
ObserverBackpressure, CorrelationDegraded, Unsupported, Cancelled }`.
`Partial` is never reported as `Complete`, even when the partial analysis is
useful.

Visibility is structured, never a `full_context` boolean:
`ContextVisibility { explicit_request_complete, uses_previous_response,
uses_conversation_state, uses_item_references, uses_prompt_reference,
uses_external_files, uses_external_images, contains_opaque_items,
logical_context_status }` with `LogicalContextStatus { ExplicitOnly,
ProviderManagedPartial, ExternalReferencesPartial, MixedPartial, Unknown }`.
Tracepress only claims to know the explicit bytes it observed; it never
claims the model received exactly this context when provider-managed
references exist.

`ContextBlockOccurrence { id, snapshot_id, ordinal, parent_id, kind, role,
origin, locator, raw_bytes, exact_fingerprint, semantic_fingerprint,
token_estimate, detector_result }`.

`ContextBlockKind { Instructions, Message, Text, ImageReference,
FileReference, ToolDefinition, ToolCall, ToolResult, ItemReference,
PromptReference, ProviderStateReference, AssistantHistory, OpaqueReasoning,
Opaque, Unknown }` — extensible; a new provider item type must not break
anything.

`ContextRole { System, Developer, User, Assistant, Tool, Unknown }` is
separate from the kind. `ContextOrigin { HumanAuthored, AgentGenerated,
ToolGenerated, ToolSchema, ProviderManaged, ExternalReference,
TracepressGenerated, Unknown }` is observed only; it does not drive any
decision in this phase.

## 12–17. Raw-span indexing

Phase 3 builds an index of spans over the original request bytes.
`BlockLocator { semantic_path, raw_value_start, raw_value_end, occurrence }`.
The raw span, not the semantic path, is the authoritative structural
identity inside a request, because duplicate keys make a path ambiguous.

This exists now so Phase 4 can replace only a tool-result payload without
parse → modify → reserialize, which would change whitespace, key ordering,
escaping, number representation, and unrelated bytes.

Duplicate keys are recorded with `duplicate_key_detected = true` and an
occurrence index; genuine ambiguity yields `Partial`. Forwarding never
changes.

Span invariants (property tested): `0 <= start <= end <= request.len`;
every child span is contained in its parent; sibling payload spans do not
overlap unless the structural model explicitly allows it; and
`request[start..end]` is exactly the identified JSON value.

Mandatory span cases: minified JSON, pretty JSON, CRLF, tabs, escaped
quotes, escaped slash, `\uXXXX`, surrogate pairs, emoji UTF-8, empty
strings, empty arrays, nested arrays, nested objects, duplicate keys,
unknown fields, NUL escape, large strings, `input` as string, `input` as
item array.

## 18–21. Execution model

No parsing on the forwarding task: the span indexer and the analyzer work on
a bounded observer copy, off the forwarding path. No `await analyzer` before
forwarding.

Ingestion is batched, because one request can produce thousands of blocks:
`BeginContextAnalysis`, `AppendContextBlocks { analysis_id, sequence,
blocks[] }`, `FinalizeContextAnalysis { analysis_id, summary }`. Every batch
respects the existing IPC bound.

A crash between `Begin` and `Finalize` must reconstruct as `Partial` or be
removed by deterministic recovery. A snapshot never appears `Complete`
without its `Finalize`.

Explicit analysis limits: max request bytes analyzed, max blocks, max JSON
depth, max string bytes inspected, max analysis work, max analysis wall
time, max batches. Exceeding one yields `ResourceLimit`, persists the
aggregates already available, and forwarding continues.

## 22–24. Fingerprints

Exact fingerprint: `SHA-256(raw span bytes)`, detecting byte-identical
repetition. Semantic fingerprint: optional and versioned,
`SHA-256(fingerprint_version + block_kind + role + decoded semantic
content)`, detecting equivalent content across whitespace, wire formatting,
and equivalent escaping.

Semantic fingerprints are implemented only where equivalence is definable
with confidence — initially `Text`, tool-result strings, and message text —
and for JSON only under a clearly defined, tested canonicalisation. JSON
with duplicate keys is never canonicalised as if unambiguous. When unsafe,
`semantic_fingerprint = NULL`. `fingerprint_version` is persisted; an
unversioned semantic fingerprint is never used for training.

## 25–31. Token estimation and reconciliation

`trait TokenEstimator { fn estimate(&self, model: &str, content: &[u8]) ->
TokenEstimateResult; }` producing `TokenEstimate { tokens, estimator,
estimator_version, encoding, confidence }` with `EstimateConfidence {
ModelMapped, GenericTokenizer, Heuristic }`. No estimate is ever called
exact. `bytes/4` is not truth: without a known tokenizer/model mapping,
prefer `NULL` or a fallback explicitly labelled `Heuristic`, and never mix it
with real usage.

`ProviderObservedTokens` (from provider usage) and
`LocallyEstimatedTokens` (from the analyzer) are never summed as one source.

`TokenReconciliation { snapshot_id, visible_estimated_tokens,
provider_input_tokens, residual_tokens, comparability }` with
`ReconciliationStatus { ComparableApproximate, PartialVisibility,
MissingProviderUsage, MissingLocalEstimate, NotComparable }`. A residual is
reported as a residual, never as "hidden tokens"; it can come from provider
protocol overhead, provider-managed state, files/images, tokenizer mismatch,
hidden transformations, or local estimation error. A negative residual is
valid, is never clamped to zero, and is recorded as estimator-accuracy
evidence.

## 32–36. Composition, repetition, stable prefix

Per snapshot: `explicit_bytes`; estimated tokens by kind, role, and origin;
tool-definition, tool-result, human-text, and assistant-history shares; and
unique versus repeated content share. Every token-derived value is marked
`estimated`.

Within a session, identify the same exact block, the same semantic block,
new, changed, and removed blocks — via fingerprint indexes, never an O(n²)
text comparison. `ContextDelta { previous_snapshot_id, current_snapshot_id,
repeated_blocks, new_blocks, changed_blocks, removed_blocks,
repeated_estimated_tokens, new_estimated_tokens, common_prefix_blocks,
common_prefix_estimated_tokens }`.

The stable explicit prefix is called `stable_explicit_prefix_estimate`, never
`cacheable_tokens`. It may be correlated with the provider's observed
`cached_tokens` as analysis only; the two values are stored separately and
are never equated.

## 37–42. Provider-managed and external content

Detect `previous_response_id`, `conversation`, item references, and prompt
references, and set `logical_context_status = ProviderManagedPartial` where
applicable. When a `previous_response_id` matches a response Tracepress
observed, record a logical `provider_state_reference` edge and
`reference_resolved_locally = true`, otherwise `false` — without assuming the
provider's effective context can be reconstructed byte for byte.

Never fabricate history by concatenating old request + old response + new
request and calling it provider context; keep it as
`locally reconstructable history`, separate from the explicit current
request.

For `file_id`, `file_url`, `image_url`, and prompt references: no download,
fetch, OCR, or external resolution. Record reference type, presence, safe
metadata, and partial visibility, without persisting full sensitive URLs
absent an explicit policy.

Tool definitions are measured separately (function schemas, provider
built-in tools, MCP declarations) as `tool_count`, `schema_bytes`,
`estimated_schema_tokens`, `largest_tool_schema`, `repeated_schema_tokens`,
without automatically persisting full schemas. For each tool result record
`tool_call_id`, tool name when correlatable, bytes, estimated tokens,
detector result, exact and semantic repetition, and line count. Nothing is
compressed.

## 43–49. Detection, features, opportunity signals

`trait ShadowContentDetector { fn detect(&self, content: &[u8], metadata:
&BlockMetadata) -> DetectionResult; }` producing `DetectionResult { kind,
confidence, detector_version }` over `DetectedContentKind { Json, Ndjson,
Log, SearchResults, TestResults, SourceCode, Diff, PlainText, BinaryLike,
Unknown }`. Detection activates no compressor; it produces features.

No ML. Only strong structural signals: valid JSON parse, NDJSON parse ratio,
log timestamp/level patterns, `file:line` grep patterns, test-runner
structure, unified-diff markers, source-language heuristics. Every detector
can abstain with `Unknown`; `Unknown` is preferred over a false
classification.

Bounded features: raw bytes, line count, max line length, duplicate and
unique line ratios, JSON item count and depth, error and warning line
density, repetition score, detected kind. Aggregates only — never one row per
line.

`OpportunitySignal { LargeToolResult, HighDuplication, HomogeneousJson,
RepetitiveLogs, LargeSearchResult, LargeTestOutput, LargeToolSchema,
RepeatedHistory }` are signals, not decisions. No
`target_compression_ratio`, `compressor`, or `tokens_saved`.
`candidate_estimated_tokens` is persisted when estimable;
`estimated_tokens_saved` is not, until a real candidate transformation exists
in Phase 4.

## 50–55. Privacy, compactness, missingness

Default is context-analysis metadata only. SQLite receives no prompt text,
tool result text, function arguments, tool schema contents, URLs, or file
content, except under the already-planned explicit capture mode.
Fingerprints stay local; a future dataset export uses
`HMAC(local_secret, fingerprint)`, never a raw deterministic SHA.

A normal block observation is far below the IPC frame bound. Long metadata
strings are truncated and hashed; content required for forwarding is never
touched.

Drop accounting: `analysis_requests_seen`, `analysis_requests_complete`,
`analysis_requests_partial`, `analysis_requests_dropped`,
`analysis_drop_backpressure`, `analysis_drop_resource_limit`,
`analysis_drop_malformed`, `analysis_drop_correlation`. Coverage:
`analysis_coverage = fully_analyzed_requests / eligible_requests`, plus
`correlation_coverage`, `token_estimation_coverage`, and
`semantic_detection_coverage`.

Every future training row must distinguish `feature = 0` from
`feature = unknown`, and `content absent` from `content not observed`. That
semantics is preserved now.

## 56–58. Persistence

Create `0003_context_analysis.sql`; never modify `0001` or `0002`. Inspect
the current schema first and reuse `content_objects`,
`content_occurrences`, `provider_requests`, `provider_usage`, `events`, and
`operations` where they fit semantically. Prefer integration over
duplication: check whether `content_occurrences` can already represent part
of `context_block_occurrences`.

Conceptually needed: `context_snapshots`, `context_block_occurrences`,
`context_analysis_metrics`, `context_deltas`, `token_reconciliations`.

New event types: `context.analysis.started`, `context.analysis.completed`,
`context.analysis.partial`, `context.analysis.dropped`,
`context.correlation.degraded`, `context.visibility.partial`,
`context.reconciliation.completed`,
`context.reconciliation.unavailable` — no sensitive payloads.

## 59–60. Performance and memory

Phase 3 must not reintroduce the hot-path parsing problem fixed in Phase 2.
Mandatory benchmark of Phase 2 baseline forwarding versus Phase 3
forwarding, measuring dispatch latency, proxy TTFB and TTFT overhead, RSS,
and queue depth. No arbitrary target is fixed before measuring. Gate: Phase
3 may not cause a significant forwarding regression against the Phase 2
baseline; a regression requires a performance investigation before closing.

Giant requests are never duplicated repeatedly: `Bytes` clones, bounded tap,
streaming hash, bounded semantic buffer. Beyond the semantic bound, forward
everything and analyse partially or skip.

## 61–62. CLI

`tracepress context <request-id>` shows visibility, provider input observed,
visible estimate with estimator and reconciliation, estimated composition,
repetition, stable explicit prefix, largest blocks, analysis status, and
correlation status. Content is not shown by default. Optionally, if scope
stays contained, `tracepress context stats <session-id>` shows per-request
evolution. No dashboard.

## 63. Task breakdown

```text
T1  Design contracts (docs/design/phase-3-shadow-context-analysis.md)
T2  Correlation degradation observability
T3  Context canonical domain
T4  Migration 0003
T5  Raw JSON span indexer (independent review before advancing)
T6  Responses context extractor
T7  Visibility analyzer
T8  TokenEstimator abstraction
T9  Token reconciliation
T10 Fingerprints + context deltas
T11 Shadow content detectors
T12 Feature extraction
T13 Batched IPC + durable integration
T14 Context CLI
T15 Adversarial + privacy + performance
T16 Independent review (>= 2 reviews, P0/P1 closed with regression tests)
```

## 64. Critical named tests

Provider-managed context (`previous_response_id` ⇒
`ProviderManagedPartial`, never `ExplicitOnly`); prompt reference ⇒ partial
visibility; file/image references not downloaded and marked
partial/external; the same tool result repeated ⇒ same exact fingerprint and
counted as repeated; the same text with different JSON escaping ⇒ same
semantic fingerprint, different exact fingerprint; duplicate JSON key ⇒
occurrence preserved, span correct, anomaly recorded; request above the
analysis budget ⇒ provider receives every byte and analysis is
`ResourceLimit`; IPC queue full ⇒ the provider request continues, analysis is
`ObserverBackpressure`, coverage decreases; 5000 context blocks ⇒ no frame
above the IPC bound, batches bounded; crash before `Finalize` ⇒ the snapshot
is not `Complete`.

## 65. Property tests

```text
analyze(raw) does not modify raw
every span lies within the body
fragmenting the observer input does not alter the semantic result
the same exact block yields the same exact fingerprint
a fixed semantic fingerprint version is deterministic
missing usage never fabricates a token count
a provider-managed reference never yields full/explicit logical context
```

## 66. Fuzzing

New targets reusing the Phase 2 fuzz CI: context span indexer, Responses
context extractor, semantic fingerprint, content detector, context delta
builder. A failure corpus is versioned when an important regression appears.

## 67. Definition of Done

Phase 3 is closed only when:

1. Forwarding is still byte-exact.
2. No compression exists.
3. A `ContextSnapshot` can be created per eligible request.
4. Every observed block has a position and a type.
5. Spans point at the correct bytes of the original request.
6. Duplicate JSON keys do not corrupt the index.
7. Explicit context is distinguishable from provider-managed/external.
8. `previous_response_id`/conversation/references never produce false full
   visibility.
9. Tokens per block are estimated when a supported estimator exists.
10. Every estimate identifies estimator, version, and confidence.
11. Provider usage remains the aggregate source of truth.
12. Reconciliation never turns estimates into billing truth.
13. Negative residuals are preserved.
14. Context composition is measurable.
15. Intra- and inter-request repetition is measurable.
16. The stable explicit prefix is measurable.
17. Large tool results and tool definitions are identifiable.
18. Candidate tool-output kinds are detected without compressing them.
19. Correlation degradation is explicitly observed.
20. Analysis drops are quantified.
21. Dataset coverage is computable.
22. No raw content travels inside large IPC records.
23. Batching supports requests with many blocks.
24. A crash during analysis never produces a falsely complete snapshot.
25. Secrets appear in no database row, log, or event payload.
26. Raw content is not persisted by default.
27. Giant inputs stay memory-bounded.
28. Analyzer backpressure never blocks forwarding.
29. Performance does not regress significantly against Phase 2.
30. Every Phase 0/1/2 test still passes.
31. Migration 0003 is additive.
32. `0001` and `0002` stay byte-identical.
33. Fuzz targets compile and CI runs them under the existing scheme.
34. Independent reviews leave no P0/P1 open.

## 68. Metrics Phase 3 must be able to produce

Provider input observed; explicit context analysed coverage; visible token
estimate; composition shares by kind; repeated explicit context; stable
explicit prefix; detected tool-output kind distribution; share of
provider-managed requests; and reconciliation status distribution. These
figures never state "Tracepress can save 61.2%", only "61.2% of the
estimated visible context came from tool results".

## 69. Gate for Phase 4

Phase 4 does not start because Phase 3 compiles. First run Phase 3 over
enough real sessions to answer which categories actually consume the most
tokens, what share is tool output, what share is tool definitions, how much
content repeats, how many requests use provider-managed state, which
detectors have good coverage and confidence, how large candidate blocks are,
how much reconciliation error exists, and how much dataset is lost to
backpressure and correlation degradation. Only then are the first
compressors selected. Do not assume logs, JSON, grep, or tests are the
highest ROI; let the Phase 3 data decide.

## 70. Phase 4, planned but not implemented

The next gate is `Phase 4 — Shadow Compression & Reversible Rendering`,
where a raw block becomes a candidate representation with stored
recovery and hypothetical token measurement — still in shadow, the candidate
not sent to the provider, until fidelity and recovery are validated.
