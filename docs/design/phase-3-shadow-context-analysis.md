# Tracepress Phase 3 Shadow Context Analysis Contract

Status: approved implementation contract. Frozen before Phase 3 production code. Baseline `2b5fd6f` (`phase-2-complete`).

## Scope

Phase 3 decomposes the explicit bytes of an observed `POST /v1/responses` request into typed, positioned, measured blocks, correlates them with the provider usage already captured in Phase 2, and records how much of the request is invisible to Tracepress. It observes, classifies, measures, and correlates. It performs no transformation, compression, rewriting, fetching, or request mutation.

Out of scope: compression, candidate rendering, recovery, pricing, policy, ML, embeddings, dataset export, Parquet, DuckDB, dashboards, a second provider protocol, and any claim of tokens saved.

`/v1/chat/completions` stays transport-only and is never context-analysed.

## Placement

A new crate `tracepress-context` owns the analysis domain: canonical types, the raw JSON span indexer, the Responses v1 context extractor, fingerprints, the token-estimator abstraction, content detectors, feature extraction, and delta construction. It depends on `tracepress-core` (ids, limits, content ids) and on `tracepress-provider` for `ProviderKind`/`ProviderProtocol` only.

`tracepress-provider` is not extended with context decomposition: its parsers answer "what did the provider report", while context analysis answers "what did we send". Keeping them apart is what lets Phase 4 rewrite a request payload without touching usage/lifecycle parsing.

`tracepress-proxy` runs the analyzer inside the existing detached blocking observation task and hands the result to a sink. `tracepress-cli` batches it over IPC. `tracepress-daemon` owns identity allocation and durable writes. `tracepress-storage` gains migration `0003`.

## Transparency and execution model

The forwarding task performs no analysis and never awaits one. Analysis runs on the same detached `spawn_blocking` task that already parses the request semantically, over the refcounted `Bytes` handle of the accepted request buffer. The externally observable invariants of Phase 2 hold unchanged:

```text
forwarded_request_bytes  == original_request_bytes
forwarded_response_bytes == upstream_response_bytes
```

Analysis may complete, be partial, hit a resource limit, find malformed input, be dropped by bounded-queue backpressure, be degraded by correlation, be unsupported, or be cancelled. None of those outcomes may alter bytes, status, headers, or forwarding progress. Every sink and IPC call is non-blocking and fail-open.

`analyze(raw)` never mutates `raw`; the analyzer takes a shared slice and owns no mutable view of the request.

## Canonical domain

`ContextSnapshotId` and `ContextBlockOccurrenceId` are UUIDv7, allocated by the daemon, consistent with every other externally visible id.

`ContextSnapshot` carries `id`, `session_id`, `provider_request_id`, `inference_operation_id`, `analysis_version`, `status`, `visibility`, `request_content_hash`, `explicit_block_count`, `analyzed_bytes`, `skipped_bytes`, `started_at_us`, `completed_at_us`. `analysis_version` is `1` for this phase and is persisted with every snapshot so a future re-analysis is distinguishable rather than silently mixed.

`request_content_hash` is `SHA-256` over the exact accepted request bytes, computed by the analyzer without retaining the bytes.

`ContextAnalysisStatus` is `Complete | Partial | ResourceLimit | Malformed | ObserverBackpressure | CorrelationDegraded | Unsupported | Cancelled`. A partial analysis with useful aggregates is still `Partial`.

`ContextVisibility` is a struct of explicit booleans plus `LogicalContextStatus` (`ExplicitOnly | ProviderManagedPartial | ExternalReferencesPartial | MixedPartial | Unknown`). No boolean named `full_context` exists anywhere. `ExplicitOnly` requires that the request declares no `previous_response_id`, no conversation state, no item reference, no prompt reference, no external file or image, and no opaque item. Any one of those forces the corresponding partial status; several force `MixedPartial`.

`ContextBlockOccurrence` carries `id`, `snapshot_id`, `ordinal`, `parent_id`, `kind`, `role`, `origin`, `locator`, `raw_bytes`, `exact_fingerprint`, `semantic_fingerprint`, `token_estimate`, `detector_result`. `ordinal` is the document order of the block within its snapshot and is stable for identical bytes.

`ContextBlockKind`, `ContextRole`, and `ContextOrigin` are non-exhaustive enums with an `Unknown` member. An unrecognised provider item type maps to `Unknown` with its span and byte count still recorded; it never aborts the analysis and never degrades a snapshot below `Partial`.

Role is orthogonal to kind: a tool result is `kind = ToolResult, role = Tool`, never a `Message` with an ad-hoc string.

## Raw-span indexing

The indexer is a bounded, allocation-disciplined JSON scanner over the original bytes. For every value it reports it produces a byte range that is exactly that JSON value: for a string, the range includes its enclosing quotes; for an object or array, its enclosing braces or brackets; for a scalar, its literal text. No unescaping, normalisation, or re-encoding participates in span computation.

`BlockLocator` carries `semantic_path`, `raw_value_start`, `raw_value_end`, and `occurrence`. The raw span is the authoritative structural identity within a request. `semantic_path` is a JSON-Pointer-shaped convenience string, bounded in length, and is never treated as unique.

Invariants, property tested:

```text
0 <= raw_value_start <= raw_value_end <= request.len()
child span ⊆ parent span
sibling payload spans do not overlap
request[start..end] is exactly the identified JSON value
```

Duplicate object keys are recorded rather than resolved: each colliding member gets its own occurrence index, `duplicate_key_detected` is set on the snapshot, and a structure whose block identity becomes genuinely ambiguous yields `Partial`. Forwarding is unaffected in every case.

Escaped content — `\"`, `\\`, `\/`, `\uXXXX`, surrogate pairs, an escaped NUL, multi-byte UTF-8 — affects the decoded value, never the span. Raw invalid UTF-8 in the body is `Malformed` for analysis, consistent with the Phase 2 parsers, and still forwarded.

The indexer is the highest-risk component of this phase and requires an independent review before dependent slices are considered complete.

## Responses v1 extraction

The extractor maps the request document to canonical blocks: `instructions`; `input` as a bare string or as an item array; message items and their content parts; function calls and function call outputs; tool definitions including provider built-in tools; item, prompt, and conversation references; `previous_response_id`; file and image references; opaque and reasoning items; and anything unrecognised as `Unknown`.

Extraction is tolerant: unknown members are ignored for typing but still contribute their span and byte count to the parent block, so `analyzed_bytes` remains honest. A known member with an impossible shape degrades the snapshot to `Partial` and never fails forwarding.

Origin is assigned structurally: a user message is `HumanAuthored`, an assistant message or tool call is `AgentGenerated`, a function call output is `ToolGenerated`, a tool schema is `ToolSchema`, a provider reference is `ProviderManaged`, a file or image reference is `ExternalReference`, and anything else is `Unknown`. Origin is recorded only; nothing in this phase branches on it.

## Provider-managed and external references

A `previous_response_id` matching a `provider_attempts.provider_response_id` already observed in the same session records `reference_resolved_locally = true` and a logical `provider_state_reference` edge between the two inference operations; otherwise `false`. Neither case licenses a claim about the provider's effective context, and no history is fabricated by concatenating earlier requests and responses.

External references are never fetched, downloaded, resolved, or OCR'd. Only the reference kind, its presence, and bounded safe metadata are recorded. A URL is not persisted; a bounded, salted-free hash of it plus its scheme and host length class is the maximum recorded, and even that only as metadata.

## Fingerprints

The exact fingerprint is `SHA-256` over the raw span bytes and detects byte-identical repetition.

The semantic fingerprint is optional and versioned: `SHA-256(fingerprint_version || block_kind || role || decoded semantic content)`. It is produced only for `Text`, message text, and tool-result string payloads, where decoding a JSON string to its characters is an unambiguous equivalence. It is not produced for JSON documents, for any block whose subtree contains duplicate keys, or for any block whose decoding was truncated by a limit; in those cases it is `NULL`.

`fingerprint_version` is persisted with every semantic fingerprint. An unversioned semantic fingerprint may never be used for training or for equivalence claims.

Fingerprints stay local. A future export uses `HMAC(local_secret, fingerprint)`; a raw deterministic digest is never exported.

## Token estimation

`TokenEstimator` maps a model name plus content bytes to a `TokenEstimate` of `tokens`, `estimator`, `estimator_version`, `encoding`, and `confidence` (`ModelMapped | GenericTokenizer | Heuristic`). No estimate is ever labelled exact.

Phase 3 ships one estimator, a deterministic structural heuristic reported as `Heuristic` with an explicit name and version, and no model-to-encoding mapping. `ModelMapped` and `GenericTokenizer` remain unimplemented rather than approximated: a real tokenizer arrives with its own vetted dependency and mapping table, and until then `token_estimation_coverage` reports honestly that no model-mapped estimate exists. Where even the heuristic cannot apply — binary-like or undecodable content — the estimate is `NULL`, never zero.

`ProviderObservedTokens` and `LocallyEstimatedTokens` are distinct concepts, stored in distinct columns, and never summed together.

## Reconciliation

`TokenReconciliation` carries `snapshot_id`, `visible_estimated_tokens`, `provider_input_tokens`, `residual_tokens`, and `comparability` (`ComparableApproximate | PartialVisibility | MissingProviderUsage | MissingLocalEstimate | NotComparable`).

`residual_tokens = provider_input_tokens - visible_estimated_tokens`, signed, computed with checked arithmetic, and never clamped. A negative residual is valid evidence of estimator inaccuracy and is preserved. `ComparableApproximate` requires `LogicalContextStatus::ExplicitOnly`, a complete or partial-but-block-complete analysis, a present provider input total, and a present local estimate; provider-managed or external visibility yields `PartialVisibility` even when both numbers exist. Provider usage remains the aggregate source of truth; reconciliation never rewrites it and never becomes billing truth.

## Composition, repetition, and stable prefix

Per snapshot, `context_analysis_metrics` records `explicit_bytes`, estimated tokens grouped by kind, role, and origin, the tool-definition, tool-result, human-text, and assistant-history shares, the unique versus repeated content share, and the tool-definition aggregate (`tool_count`, `schema_bytes`, `estimated_schema_tokens`, `largest_tool_schema`, `repeated_schema_tokens`). Every token-derived value is marked as estimated by construction: it lives in a column whose name and documentation say `estimated`, alongside the estimator identity.

Within a session, repetition is computed through fingerprint indexes only. `ContextDelta` compares a snapshot against the previous snapshot of the same session and records repeated, new, changed, and removed block counts, repeated and new estimated tokens, and the common prefix in blocks and estimated tokens.

The stable prefix is named `stable_explicit_prefix_estimate` and is defined as the longest run of leading blocks, in ordinal order, whose exact fingerprints match the previous snapshot's leading run. It is never named `cacheable_tokens` and is never equated with the provider's observed `cached_tokens`; both values are stored separately so their relationship can be studied later.

## Detection and features

`ShadowContentDetector` maps content bytes plus bounded block metadata to `DetectionResult { kind, confidence, detector_version }` over `DetectedContentKind`. Detectors are deterministic structural heuristics: a valid JSON parse, an NDJSON line-parse ratio, timestamp and level patterns for logs, `path:line` patterns for search results, test-runner structure, unified-diff markers, and conservative source-language signals. Every detector may abstain; `Unknown` with a low confidence is preferred over a false positive. No ML, no model, no training.

Bounded per-block features are recorded as aggregates only: raw bytes, line count, maximum line length, duplicate and unique line ratios, JSON item count and depth, error and warning line density, and a repetition score. There is never one durable row per line.

`OpportunitySignal` values are attached to a block or a snapshot as signals. `candidate_estimated_tokens` may be recorded; `estimated_tokens_saved`, a target ratio, and a compressor choice may not exist anywhere in this phase.

## Bounded analysis

Analysis limits are explicit configuration derived from the existing `tracepress-core` resource limits, never a parallel set of invented globals:

```text
max_analyzed_bytes          min(max_request_body_bytes, 4 MiB)
max_blocks                  8192
max_json_depth              64
max_string_bytes_inspected  65536
max_analysis_work_units     derived from max_cpu_work_units
max_analysis_wall_time_ms   250
max_batches                 128
```

Crossing a bound stops that dimension of the work, sets `ResourceLimit`, and persists whatever aggregates are already complete, including `skipped_bytes`. Nothing over-bound becomes observable, and work already completed within a bound is never discarded — the Phase 2 lesson about discarding framed events applies here verbatim.

Memory discipline: one refcounted handle to the request bytes, spans instead of copies, streaming hash, and bounded decode buffers. A request above `max_analyzed_bytes` is forwarded in full and analysed partially or skipped, with `skipped_bytes` recording the difference.

## Batched ingestion

The CLI/daemon control protocol gains three messages. `BeginContextAnalysis` names the session, the provider request, and the inference operation, and returns the daemon-allocated `ContextSnapshotId`. `AppendContextBlocks` carries the snapshot id, a monotonic `sequence`, and a bounded slice of block records. `FinalizeContextAnalysis` carries the snapshot id, the terminal status, and the summary metrics, deltas, and reconciliation.

Every message is sized to fit the existing 32 KiB IPC body bound, which is not raised. The batch block count is chosen so a worst-case block record still fits, and a test pins that a full batch of maximum-size records stays under the bound. A record that cannot fit even alone is dropped with the snapshot degraded, never truncated into something that looks complete.

Block records carry ids, kinds, roles, origins, spans, byte counts, fingerprints, estimates, detector results, and features. They never carry prompt text, tool result text, function arguments, schema contents, URLs, or file content. Long metadata strings such as `semantic_path` and tool names are bounded and, when truncated, are recorded as truncated plus a hash of the full value.

A snapshot is `Complete` only after its `Finalize` commits. A crash between `Begin` and `Finalize` leaves a snapshot whose status is not `Complete`; deterministic startup recovery marks such snapshots `Partial`, in the same spirit as Phase 1 stale-session recovery.

## Correlation degradation

The Phase 2 correlation bounds stay bounded. Their degradation becomes observable: `correlation_degraded_total`, `correlation_degraded_inflight_limit`, `correlation_degraded_retired_limit`, and `correlation_missing_total`, plus the `context.correlation.degraded` event carrying reason, session id, and timestamp only. A request that cannot be fully correlated is recorded with `correlation_status = degraded` and, when it prevents analysis, `ContextAnalysisStatus::CorrelationDegraded`. Complete causality is never implied.

## Drop accounting and coverage

Counters: `analysis_requests_seen`, `analysis_requests_complete`, `analysis_requests_partial`, `analysis_requests_dropped`, `analysis_drop_backpressure`, `analysis_drop_resource_limit`, `analysis_drop_malformed`, `analysis_drop_correlation`. Coverage ratios are derived, not stored as truth: `analysis_coverage`, `correlation_coverage`, `token_estimation_coverage`, `semantic_detection_coverage`.

Missingness is preserved everywhere: `feature = 0` is distinguishable from `feature = unknown` by SQL `NULL`, and `content absent` is distinguishable from `content not observed` by the snapshot status plus `skipped_bytes`. No aggregate substitutes zero for an unknown.

## Persistence

`0003_context_analysis.sql` is additive; `0001` and `0002` are byte-identical and remain pinned by checksum tests.

Reuse was evaluated explicitly. `provider_requests`, `operations`, `provider_usage`, and `events` are reused: a snapshot references `provider_requests.request_id` and `operations.operation_id`, reconciliation joins `provider_usage`, and every new event type lands in `events`. `content_occurrences` is deliberately not reused for block occurrences: it requires a `content_objects` row, i.e. persisting raw bytes, which this phase forbids by default, and its role vocabulary is content provenance rather than context structure. Duplicating that concept would be worse than the honest separation.

New tables: `context_snapshots`, `context_block_occurrences`, `context_analysis_metrics`, `context_deltas`, `token_reconciliations`. Foreign keys prevent orphan blocks, metrics, deltas, and reconciliations. Unknown numeric values are `NULL`. Text columns hold bounded metadata only.

New event types: `context.analysis.started`, `context.analysis.completed`, `context.analysis.partial`, `context.analysis.dropped`, `context.correlation.degraded`, `context.visibility.partial`, `context.reconciliation.completed`, `context.reconciliation.unavailable`. Payloads carry identifiers, versions, statuses, counts, and timings only.

The single daemon-owned writer remains the only SQLite mutation path. A `Finalize` commits its metrics, delta, reconciliation, and terminal events in one transaction.

## Privacy boundary

The durable and IPC allowlist is exactly the structural and statistical fields named above. Prompt text, instructions, tool arguments, tool results, tool schema contents, metadata values, URLs, file contents, `user`, `prompt_cache_key`, and `safety_identifier` are excluded by construction, not scrubbed. `Debug` implementations expose kinds, counts, spans, and statuses, never content or decoded buffers. Canary suites assert absence across SQLite, events, logs, `Debug` output, and the IPC wire.

## Performance gate

A benchmark compares Phase 2 baseline forwarding against Phase 3 forwarding on identical inputs, measuring dispatch latency, proxy TTFB and TTFT overhead, RSS, and observer queue depth. No target number is fixed before measuring. Phase 3 may not cause a significant forwarding regression; if one appears, a performance investigation precedes closing the phase.

## Verification

Format, strict Clippy, workspace tests, workspace build, and the fuzz-package check all stay green. New fuzz targets cover the span indexer, the Responses context extractor, the semantic fingerprint, the content detectors, and the delta builder, reusing the Phase 2 fuzz workflow.

Two independent reviews are mandatory: one on correctness, data model, spans, missingness, and causality; one on privacy, resource limits, performance, dataset quality, and Phase 4 rewrite safety. Every P0 and P1 is closed with a regression test that fails without the fix.
