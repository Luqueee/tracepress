# Tracepress Phase 3 Shadow Context Analysis Contract

Status: approved implementation contract. Frozen before Phase 3 production code. Baseline `2b5fd6f` (`phase-2-complete`).

## Scope

Phase 3 decomposes the explicit bytes of an observed `POST /v1/responses` request into typed, positioned, measured blocks, correlates them with the provider usage already captured in Phase 2, and records how much of the request is invisible to Tracepress. It observes, classifies, measures, and correlates. It performs no transformation, compression, rewriting, fetching, or request mutation.

Out of scope: compression, candidate rendering, pricing, policy, ML, embeddings, dataset export, Parquet, DuckDB, dashboards, a second provider protocol, and any claim of tokens saved.

`/v1/chat/completions` stays transport-only and is never context-analysed.

## Placement

A new crate `tracepress-context` owns the analysis domain: canonical types, the raw JSON span indexer, the Responses v1 context extractor, fingerprints, the token-estimator abstraction, content detectors, feature extraction, and delta construction. It depends on `tracepress-core` (ids, limits, content ids) and on `tracepress-provider` for `ProviderKind`/`ProviderProtocol` only.

`tracepress-provider` is not extended with context decomposition: its parsers answer "what did the provider report", while context analysis answers "what did we send". Keeping them apart is what lets Phase 4 rewrite a request payload without touching usage/lifecycle parsing.

`tracepress-proxy` keeps Phase 2 provider observation independent, retains a context-analysis trigger with response-body state, and submits bounded Shadow work after settlement. `tracepress-cli` batches it over IPC. `tracepress-daemon` owns identity allocation and durable writes. `tracepress-storage` gains migration `0003`.

## Transparency and execution model

The selected execution mode is typed `ContextAnalysisMode { Off, Shadow }`. Production defaults to `Shadow`; `TRACEPRESS_CONTEXT_ANALYSIS=off` disables every Phase 3 context-analysis task and handoff, while `TRACEPRESS_CONTEXT_ANALYSIS=shadow` enables bounded analysis. `Off` leaves Phase 2 provider request/response observation enabled. In either mode the forwarding path preserves the exact byte identities:

```text
forwarded_request_bytes  == original_request_bytes
forwarded_response_bytes == upstream_response_bytes
```

Request observation is queued independently as soon as the request body is available. In `Shadow`, admission acquires the analyzer reservation before retaining the Phase 3 `Bytes` handle or spawning a detached Phase 3 analysis task; a failed reservation carries an explicit `ObserverBackpressure` outcome without retaining that analysis body or task. The trigger is retained alongside the response-body state and starts only after the response is settled — downstream completion, downstream drop, or upstream failure. It then observes a 10 ms quiescence window so a newly admitted forward has priority. Analysis proceeds only when no forward is active; an active forward produces the same explicit backpressure outcome. The two-slot reservation cap therefore bounds active and queued analysis bodies/tasks together, not only CPU currently executing.

The heavy context-ingestion queue is capped at four items. The Phase 2 recorder queue remains capped at 128 items, and correlation remains bounded at 64 in-flight and 256 retired identities. `BackgroundTracker` acquires a guard before each detached durable provider half is spawned. `TransportOrdering` is one FIFO dispatcher with a pending map capped at 64 and a channel capped at 64; both halves of a forward share that FIFO, and no per-forward blocking transport task fanout exists. Metadata is admitted before its latch is exposed, and the response is dispatched behind that predecessor. If either transport bound overflows, admission records `CorrelationDegradation::InFlightLimit` explicitly (using `CorrelationStatus::Degraded` for a refused response) rather than waiting or silently losing the join. After the child exits and response forwarding is stopped, shutdown drains tracked background work under a bounded 30-second timeout; forwarding never blocks on context analysis or recording.

When the 30-second drain timeout is reached, the CLI queries lightweight durable `ContextStatus` for each bounded pending analysis by request/snapshot identity before classifying it. A terminal status with `completed_at_us` set classifies `Complete` only for status `complete`, otherwise `Partial`; an active, missing, or unavailable status remains pending and is dropped only at the forced-cancellation boundary, never by blindly treating timeout as a drop.

Provider settlement never waits for context analysis. Bounded receipts and pending-context maps join a late context outcome when it arrives; an unpaired late outcome is recorded as explicit `CorrelationDegraded` or dropped with `ObserverBackpressure`, never retained without a bound.

Analysis may complete, be partial, hit a resource limit, find malformed input, be dropped by bounded-queue backpressure, be degraded by correlation, be unsupported, or be cancelled. None of those outcomes may alter bytes, status, headers, or forwarding progress. Detached sink and IPC work is fail-open, with backpressure and missing joins represented explicitly rather than by a fabricated success.

`analyze(raw)` never mutates `raw`; the analyzer takes a shared slice and owns no mutable view of the request.

## Canonical domain

`ContextSnapshotId` and `ContextBlockOccurrenceId` are UUIDv7, allocated by the daemon, consistent with every other externally visible id.

`ContextSnapshot` carries `id`, `session_id`, `provider_request_id`, `inference_operation_id`, `analysis_version`, `status`, `visibility`, `request_content_hash`, `explicit_block_count`, `analyzed_bytes`, `skipped_bytes`, `started_at_us`, `completed_at_us`, `recovered_at_us`. `analysis_version` is `1` for this phase and is persisted with every snapshot so a future re-analysis is distinguishable rather than silently mixed.

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

Crossing a bound while unprocessed work remains stops that dimension, sets `ResourceLimit`, and persists whatever aggregates are already complete, including `skipped_bytes`. Reaching an exact append cap with no suffix left does not degrade an otherwise `Complete` status. Nothing over-bound becomes observable, and work already completed within a bound is never discarded — the Phase 2 lesson about discarding framed events applies here verbatim.

Memory discipline: one refcounted handle to the request bytes, spans instead of copies, streaming hash, and bounded decode buffers. A request above `max_analyzed_bytes` is forwarded in full and analysed partially or skipped, with `skipped_bytes` recording the difference.

`max_batches = 128` is the analyzer's per-snapshot batch limit, not an ingestion-queue capacity. Runtime capacities are separate: the heavy Phase 3 ingestion queue is capped at 4, the Phase 2 recorder queue at 128, and analyzer admission at 2 concurrent non-blocking reservations, including every retained queued analysis body/task; this is distinct from `TransportOrdering`'s 64-entry map/channel bound.

## Batched ingestion

The CLI/daemon control protocol has three messages. `BeginContextAnalysis` binds `session_id`, `provider_request_id`, `inference_operation_id`, `analysis_version`, and `started_at_us`, then returns the daemon-allocated `ContextSnapshotId`. The daemon writes the initial `Partial` snapshot and `context.analysis.started` event in the same writer batch. `AppendContextBlocks` carries the snapshot id, a monotonic `sequence`, and a bounded slice of block records. `FinalizeContextAnalysis` carries the snapshot id, terminal status and timestamp, visibility, correlation, summary metrics, delta, reconciliation, and attempt data.

Every message is sized to fit the existing 32 KiB IPC body bound, which is not raised. The maximum measured serialized append batch is 30,231 bytes (30,231 / 32,768). The daemon enforces exact per-snapshot append caps (`max_blocks=8192`, `max_batches=128`) and returns cumulative receipts. Reaching either cap exactly does not by itself degrade an otherwise `Complete` snapshot: if every analyzed block was accepted, terminal status remains `Complete`. Only an unaccepted suffix — including a body-bound overflow or a singleton that cannot fit even alone — changes the terminal status to `ResourceLimit`; the accepted prefix is persisted and no record is truncated into something that looks complete.

Block records carry bounded structural identifiers and measurements: ids, kinds, roles, origins, spans, byte counts, fingerprints, estimates, detector results, and features. Semantic paths, tool names, and tool-call ids are bounded structural identifiers and may be carried when present; when truncated, they are recorded as truncated plus a hash of the full value. Records never carry prompt text, instructions, unallowlisted content metadata values, tool result text, function arguments, schema contents, URLs, or file content.

`FinalizeContextAnalysis` commits metrics, delta, reconciliation, outcome, and applicable visibility, reconciliation, terminal, and correlation events in one writer transaction/batch. A crash after `BeginContextAnalysis` and before `FinalizeContextAnalysis` leaves `status = Partial` and `completed_at_us = NULL`. Startup recovery runs in one writer transaction: it selects only rows with `completed_at_us IS NULL AND recovered_at_us IS NULL`, changes each to `Partial`, sets `recovered_at_us`, and appends one `context.analysis.partial` event atomically. It returns a typed `RecoveryReceipt { recovered_sessions, recovered_context_snapshots }` with the counts kept separate; the `recovered_at_us` predicate/update marker makes repeated recovery idempotent. During normal shutdown, `FinishSession` first finalizes every active in-memory analysis as `Partial` and removes it from the active map before changing the session state, so no active snapshot is left unmarked.

## Metadata-only context inspection

`tracepress context <request-id>` returns bounded metadata sections: `Context visibility`; `Provider input observed`; `Visible estimate + estimator + reconciliation/residual`; `Composition (estimated)`; `Repetition`; `Stable explicit prefix`; `Largest blocks`; `Analysis`; and `Correlation`. `Largest blocks` is limited to the top-10 largest blocks by raw-byte size.

The maximum measured serialized inspection response is 5,859 bytes (5,859 / 32,768). It contains metadata only: no prompt or instruction text, tool arguments or results, schemas, URLs, file contents, decoded buffers, or other request content.

## Correlation degradation

The Phase 2 correlation bounds stay bounded. Their degradation becomes observable: `correlation_degraded_total`, `correlation_degraded_inflight_limit`, `correlation_degraded_retired_limit`, and `correlation_missing_total`, plus the `context.correlation.degraded` event carrying reason, session id, and timestamp only. A request that cannot be fully correlated is recorded with `correlation_status = degraded` and, when it prevents analysis, `ContextAnalysisStatus::CorrelationDegraded`. Complete causality is never implied.

`analysis_requests_seen` increments once when an eligible Shadow request reaches recorder admission. `analysis_requests_complete` increments once only after successful `FinalizeContextAnalysis` with `Complete`; `analysis_requests_partial` increments once after successful terminal persistence with any non-`Complete` status, including resource-limit, malformed, observer-backpressure, correlation-degraded, unsupported, cancelled, recovered-partial, and shutdown-finalized outcomes. The terminal snapshot status and `context.analysis.partial` event carry each persisted partial reason. `analysis_requests_dropped` increments once when no successful terminal persistence exists, including rejection before `BeginContextAnalysis`, explicit abort, or abandonment/forced cancellation after pending-status reconciliation; an explicit `AbortContextAnalysis` remains in this dropped partition even when it writes a terminal drop-status snapshot/event. `analysis_drop_backpressure`, `analysis_drop_resource_limit`, `analysis_drop_malformed`, `analysis_drop_correlation`, `analysis_drop_unsupported`, and `analysis_drop_cancelled` are the pre-persistence dropped-event counters (no successful terminal finalization), not persisted partial-reason counters; the corresponding `context_*_total` values retain aggregate typed outcome totals across both paths. After drain, the exact lifecycle partition is `analysis_requests_seen = analysis_requests_complete + analysis_requests_partial + analysis_requests_dropped`, with each identity counted once.

Coverage is reported as bounded ratios and is `unknown` when its denominator is zero. `analysis_coverage = analysis_requests_complete / analysis_requests_seen`; `analysis_coverage_complete = analysis_requests_complete / analysis_requests_seen`; `analysis_coverage_partial = analysis_requests_partial / analysis_requests_seen`; and `analysis_coverage_dropped = analysis_requests_dropped / analysis_requests_seen`. `correlation_coverage = correlation_correlated / correlation_eligible`, where both counts come from persisted terminal block/snapshot evidence eligible for correlation. `token_estimation_coverage = token_estimation_observed / token_estimation_eligible`, where `token_estimation_eligible` counts only accepted persisted blocks with `MeasurementApplicability::Eligible` for token estimation and `token_estimation_observed` counts those eligible blocks with a persisted estimate. `semantic_detection_coverage = semantic_detection_observed / semantic_detection_eligible`, with the same accepted-persisted-block restriction and `MeasurementApplicability::Eligible` detector applicability; `Unknown` is an observed detector result when the detector ran and persisted it. `Eligible` means a valid bounded content view exists even if a later work or string budget leaves the observation absent; `Ineligible` reference/opaque kinds and `Unavailable` inspectable malformed/unreadable views are excluded from both denominators.

Missingness is preserved everywhere: `feature = 0` is distinguishable from `feature = unknown` by SQL `NULL`, and `content absent` is distinguishable from `content not observed` by the snapshot status plus `skipped_bytes`. No aggregate substitutes zero for an unknown.

## Persistence

`0003_context_analysis.sql` is additive; `0001` and `0002` are byte-identical and remain pinned by checksum tests.

Reuse was evaluated explicitly. `provider_requests`, `operations`, `provider_usage`, and `events` are reused: a snapshot references `provider_requests.request_id` and `operations.operation_id`, reconciliation joins `provider_usage`, and every new event type lands in `events`. `content_occurrences` is deliberately not reused for block occurrences: it requires a `content_objects` row, i.e. persisting raw bytes, which this phase forbids by default, and its role vocabulary is content provenance rather than context structure. Duplicating that concept would be worse than the honest separation.

New tables: `context_snapshots`, `context_block_occurrences`, `context_analysis_metrics`, `context_deltas`, `token_reconciliations`. Foreign keys prevent orphan blocks, metrics, deltas, and reconciliations. Unknown numeric values are `NULL`. Text columns hold bounded metadata only.

New event types: `context.analysis.started`, `context.analysis.completed`, `context.analysis.partial`, `context.analysis.dropped`, `context.correlation.degraded`, `context.visibility.partial`, `context.reconciliation.completed`, `context.reconciliation.unavailable`. Payloads carry identifiers, versions, statuses, counts, and timings only.

The single daemon-owned writer remains the only SQLite mutation path. `BeginContextAnalysis` commits the initial `Partial` snapshot and `context.analysis.started` event in one batch. `FinalizeContextAnalysis` commits metrics, delta, reconciliation, outcome, and applicable terminal/visibility/reconciliation/correlation events in one transaction; no snapshot is `Complete` before this finalization.

`context.analysis.dropped` makes every explicit loss observable without reclassifying it as a partial. `RecordContextAnalysisDropped` is the aggregate form for losses with no snapshot; `AbortContextAnalysis` may emit the per-snapshot form after `BeginContextAnalysis` when an active analysis cannot continue. A successful `FinalizeContextAnalysis` with a non-`Complete` status is `Partial` and emits `context.analysis.partial`; a crash-incomplete begun snapshot is likewise recovered or shutdown-finalized as `Partial`.

## Privacy boundary

The durable and IPC allowlist is exactly the structural and statistical fields named above. Bounded semantic paths, tool names, and tool-call ids are allowlisted structural identifiers, not content. Prompt text, instructions, unallowlisted content metadata values, tool arguments, tool results, tool schema contents, raw URLs, file contents, `user`, `prompt_cache_key`, and `safety_identifier` are excluded by construction, not scrubbed; the only reference-URL exception is the bounded, salted-free hash plus scheme and host-length class described above. `Debug` implementations expose kinds, counts, spans, and statuses, never content or decoded buffers. Canary suites assert absence across SQLite, events, logs, `Debug` output, and the IPC wire.

## Performance gate

The final benchmark uses `scripts/benchmark_phase3.py` with the exact `phase-2-complete` baseline (`2b5fd6f`), current `TRACEPRESS_CONTEXT_ANALYSIS=off`, and current `TRACEPRESS_CONTEXT_ANALYSIS=shadow` arms on the deterministic local HTTP upstream. It builds the release profile with `--locked`, takes 3 warmups and 15 samples per workload, and includes the gap0 priority cases (`post_response_settle_seconds = 0`). The decision uses bootstrap confidence intervals and measured A/A envelopes, not an arbitrary fixed threshold. Request errors are required to be zero. In addition, every current arm must expose Phase 2 provider-request evidence; Shadow must report a positive `analysis_requests_seen` partition satisfying `analysis_requests_seen = analysis_requests_complete + analysis_requests_partial + analysis_requests_dropped` after drain and at least one durable terminal snapshot or explicit dropped event, while Off must report no Phase 3 snapshots or analysis events. A sequential 1 MiB Shadow workload must produce a terminal snapshot rather than passing on timing alone.

The measured ON−OFF proxy-TTFB deltas are:

| workload | ON−OFF median | measured A/A envelope |
| --- | ---: | ---: |
| small JSON | +22.910 µs | 107.730 µs |
| streamed responses | +22.661 µs | 34.650 µs |
| burst 32 | +3.500 µs | 17.245 µs |
| burst 64 | +11.125 µs | 11.490 µs |

The 1 MiB context case measured +23.120 µs and its confidence interval crosses zero. The over-budget case measured −22.380 µs and its confidence interval crosses zero. Its worst peak-minus-idle RSS delta was +34,708 KiB, bounded by the two-analysis cap. This RSS figure is the worst-case measured memory cost, not a claim of zero regression. Under the rule that each target interval must cross zero or remain within its measured A/A envelope, the operational gate was accepted only with request errors = 0 and the lifecycle checks above satisfied.

## Verification

The shipped verification covers the current green workspace result: 467 tests passed, 0 failed, and strict Clippy is clean. The Phase 3 fuzz package has five targets — `span_indexer`, `responses_extractor`, `semantic_fingerprint`, `content_detector`, and `delta_builder` — plus a finite smoke check. CI discovers standalone fuzz packages by `*/fuzz/Cargo.toml`, enumerates their targets, and runs each target under bounded time and RSS limits. The benchmark harness also has a no-op lifecycle self-test: an `on` report with zero durable snapshots/events is rejected, while valid `on` and `off` reports are accepted.

Two independent reviews are mandatory: one on correctness, data model, spans, missingness, and causality; one on privacy, resource limits, performance, dataset quality, and Phase 4 rewrite safety. Every P0 and P1 is closed with a regression test that fails without the fix.
