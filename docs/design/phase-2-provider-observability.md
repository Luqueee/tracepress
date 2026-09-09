# Tracepress Phase 2 Provider Observability Contract

Status: approved implementation contract. This document freezes the provider-observability boundary before Phase 2 production code.

## Scope and provider identity

Phase 2 observes exactly OpenAI Responses API v1 at `POST /v1/responses`. Existing `POST /v1/chat/completions` forwarding remains transport-only and byte-exact. Provider-specific knowledge lives in `tracepress-provider`; the proxy and storage layers use canonical domain values rather than provider-name string branches.

Every semantic observation carries `ProviderKind::OpenAi`, `ProviderProtocol::OpenAiResponsesV1`, and `parser_version = 1`. Normalized usage additionally carries `normalizer_version = 1`. Parser and normalizer versions are persisted so historical raw usage can be interpreted again without rewriting provenance.

Out of scope: request mutation, compression, content routing, policy optimization, pricing, telemetry export, dashboards, multiple providers, embeddings, ML, OpenTelemetry, Parquet, DuckDB, experiments, and recovery UX.

## Transparency and failure behavior

Provider observation is a side channel. Request and response bodies are forwarded from their original byte buffers or upstream chunks; semantic parsing never reserializes traffic. The externally observable invariants are:

```text
forwarded_request_bytes == original_request_bytes
forwarded_response_bytes == upstream_response_bytes
```

An observer may complete, become partial, reject unsupported input, detect malformed input, hit a resource limit, lose events to bounded-queue backpressure, or be cancelled. None of those outcomes may alter bytes, HTTP status, allowed forwarding headers, or forwarding progress. Only transport errors and the existing configured transport body limits may terminate forwarding.

The forwarding task never parses semantic input and never awaits semantic parsing or persistence. The accepted request buffer is handed to a detached blocking task through a refcounted handle, so request observation runs concurrently with upstream dispatch and reaches the sink through a non-blocking call. The requested `stream` flag is published to the response half before that sink call, so a slow sink cannot delay observer selection. Streaming response chunks go to the client first-class; the observer receives best-effort copies through a bounded `try_send` boundary and interprets them on its own task. A full or closed observer queue marks observation partial/backpressured and drops semantic work rather than slowing the stream.

A forward that never obtains an upstream response still leaves evidence. `ProviderObservationSink` accepts three records: `try_record_request`, `try_record_response`, and `try_record_transport_failure`. The last carries a `TransportFailure` classified as `Connect`, `Timeout`, `Request`, `Endpoint`, or `Other`, whose stable label states only why transport failed; it carries no endpoint URL, headers, or body bytes. Every sink call is synchronous, non-blocking, and fail-open: a rejected or dropped record never changes forwarding.

## Provider domain

`tracepress-provider` owns these non-exhaustive canonical enums:

- `ProviderKind::OpenAi`
- `ProviderProtocol::OpenAiResponsesV1`
- `ObservationStatus::{Complete, Partial, Unsupported, Malformed, ResourceLimit, ObserverBackpressure, Cancelled}`
- `ProviderResponseState::{Queued, InProgress, Completed, Incomplete, Failed, Cancelled, Disconnected, Unknown}`
- `UsageStatus::{Final, Partial, Unavailable}`
- `ContentCaptureMode::{Off, MetadataOnly, LocalRaw}`

Phase 2 defaults to `ContentCaptureMode::MetadataOnly`. `LocalRaw` is an explicit local-only capability using the existing CAS; it is never an export policy. Headers, credentials, request content, response content, metadata values, tool arguments, and identifiers such as `user`, `prompt_cache_key`, and `safety_identifier` are not part of semantic records.

The versioned `ProviderObserver` capability accepts bounded byte slices through `ObservationInput`. It produces `RequestObservation` and `ResponseObservation` values without depending on HTTP sockets or storage. `OpenAiResponsesV1Observer` is reusable offline for golden fixtures and future backfill.

## Request semantics

The Responses v1 request parser extracts only:

- model;
- `stream`, `background`, and `store` booleans;
- reasoning effort, text verbosity, and truncation mode;
- whether `previous_response_id` is present;
- bounded counts for input items, tools, and text/image/file input blocks;
- the exact length of the bounded body presented to the parser;
- provider, protocol, parser version, and observation status.

It never returns prompt text, instructions, metadata values, tool definitions or arguments, cache keys, user identifiers, headers, cookies, or credentials. Unknown JSON fields are ignored. A known field with an incompatible shape, or one whose value the observation must not retain, yields a partial observation; a body that is not one bounded JSON object with unique keys yields a malformed or resource-limit observation. Neither outcome is ever a forwarding error.

## Response lifecycle

A response observation may contain provider response ID, model, the creation value the provider itself reported, response state, incomplete reason, error code, raw usage, normalized usage, stream counters, and measured elapsed timings.

The observer for one response is selected from what the upstream declared it sent. A `text/event-stream` content type selects the incremental SSE observer; `application/json` and any `+json` media type select the bounded document parser. The request's `stream` hint breaks the tie only when the upstream declared no content type at all, and any other declared media type yields no response observation rather than feeding foreign bytes to a protocol parser. An untyped response of a request that declared no `stream` flag is likewise left unobserved. A JSON error body answering a streaming request is therefore read as a document and keeps its `error_code`; the forward's request observation is recorded either way.

Timings are local elapsed measurements in microseconds, taken from a monotonic instant captured when that observation begins: `ttfb_us` at the first non-empty upstream chunk the observer accepts, `ttft_us` at the first semantic output event, and `duration_us` when the terminal decision is reached. A buffered document is measured before it is parsed, so no timing includes the observer's own document parsing. A document response has no semantic output event, so its `ttft_us` stays absent. No wall-clock instant is derived from these measurements, and a measurement the local clock cannot represent stays absent instead of becoming a placeholder.

OpenAI status strings map to the canonical lifecycle. Missing or unknown status maps to `Unknown`; it does not imply success. A final completed event is the only semantic evidence for `Completed` in streaming mode. Client cancellation, upstream disconnect, response-limit termination, or end-of-stream without a final event must remain `Cancelled`, `Disconnected`, or `Incomplete` according to available transport evidence. Semantic evidence the provider already reported outranks a later transport decision: once a terminal lifecycle event is observed, a cancellation or disconnect keeps that state and its usage instead of discarding them.

No row is persisted per token delta and no row is persisted per lifecycle event. One settled observation commits one attempt row, one usage row when a response was observed, and its canonical events; the final lifecycle state, the stream counters, and the measured timings are the durable summary of the stream.

## Usage semantics

`RawProviderUsage` contains the exact bounded JSON bytes of the provider's `usage` object and no surrounding response content. That object is the top-level `usage` member of the response body, or of the `response` object carried by an SSE event; a `usage` key nested inside any other member — including one the provider echoes back from the client's own request — is never selected, and a `usage` string that is not a member key never suppresses the real object. `NormalizedUsage` contains optional unsigned values:

- `input_total`
- `input_cached`
- `input_uncached`
- `cache_write`
- `output_total`
- `output_reasoning`
- `total`
- `status`
- `normalizer_version`
- anomaly flags

OpenAI mappings are:

```text
input_total      = usage.input_tokens
input_cached     = usage.input_tokens_details.cached_tokens
input_uncached   = input_total - input_cached, only when cached <= total
cache_write      = usage.input_tokens_details.cache_write_tokens, else cache_write
output_total     = usage.output_tokens
output_reasoning = usage.output_tokens_details.reasoning_tokens
total            = usage.total_tokens
```

Missing values stay `None`/SQL `NULL`; they never become zero. Arithmetic uses checked operations. `input_cached > input_total`, `output_reasoning > output_total`, a reported total inconsistent with the reported components, invalid numeric types, negative and fractional numbers, and overflow set explicit anomaly metadata while preserving raw usage. A magnitude beyond the durable signed 64-bit counter range is an overflow anomaly and never becomes a component, so no provider-chosen number can make the durable write unsatisfiable. None of them break forwarding.

`Final` usage requires a terminal response state together with a usage object that yielded at least one recognised component and no anomaly. The terminal states are `Completed`, `Incomplete`, `Failed`, and `Cancelled`; `Queued`, `InProgress`, `Disconnected`, and `Unknown` are not terminal, and usage that no terminal state confirms is `Partial`. A usage object yielding no recognised component is `Unavailable`, and components extracted with an anomaly are `Partial`. An absent or null `usage` member leaves usage `Unavailable`; a member that is not an object, or whose exact bytes cannot be recovered intact, contributes no components and no raw usage and makes the observation partial. A usage object beyond the retained usage bound is refused by the parser and makes the observation a resource-limit observation, so an over-bound object never reaches the durable write.

## Incremental SSE model

The SSE framer accepts arbitrary chunk boundaries, CRLF or LF line endings, comments, optional `event` fields, and multi-line `data` fields. It retains at most the configured event-byte bound and configured event count. It never concatenates the full response stream.

Each completed SSE event is interpreted independently until terminal evidence arrives, its payload read under the same bounded JSON rules as a whole document. Known Responses v1 lifecycle events drive a small state machine; an unknown event name never moves the lifecycle and never degrades the observation, although its payload may still supply allowlisted identifiers and a usage object. A payload the bounded parser refuses degrades the observation only when it belonged to a lifecycle event. Usage is taken from the top-level `usage` member of the event payload, or of the payload's `response` member when one is present, and, before terminal evidence, a later valid usage object replaces an earlier one.

Terminal provider evidence is final, and equally final for every terminal state. The first `response.completed`, `response.incomplete`, `response.failed`, `response.cancelled`, or bare `error` event ends semantic interpretation of the stream at that event, whichever fragment carried it and whatever follows it in that same fragment. After it no event contributes anything: not a lifecycle state, an identifier, a model, an incomplete reason, an error code, a first-token measurement, nor a replacement usage object. The identity, reason, error code, and usage a terminal event reported are what the observation carries; a later frame never walks them back and never overwrites them.

The observer records:

- measured microseconds to the first upstream byte, to the first output delta or output item event, and to the terminal decision, taken from a monotonic instant captured when observation starts;
- chunk and byte counts with checked arithmetic, accruing for every fragment the owner delivers: the provider's terminal event closes semantics, not transport accounting, which stops only when the owner finishes, cancels, or disconnects the stream. A trailing `data: [DONE]` frame, a keep-alive comment, and a stray newline after the terminal event are therefore counted in `chunk_count` and `byte_count` while contributing no semantics;
- final response state and usage status.

A measurement no local clock can take stays absent; a placeholder instant is never recorded. TTFB, TTFT, and duration remain nullable when the corresponding evidence is absent. Fragmenting identical SSE bytes differently must produce identical semantic output. A framing bound therefore never discards observation work already completed within the bound: the events completed before it are interpreted, the event that crossed it is refused and never interpreted, framing stops there for the rest of the stream, and the crossed bound reaches the settled observation as a resource-limit status — reported to the owner by whichever call follows the crossing, so the fragmentation decides only when the owner hears about it.

## Resource limits

Phase 2 reuses `tracepress-core::ResourceLimits`. The semantic byte bounds are derived from the existing configured maximum request and response body bytes, so no semantic parser accepts more bytes than transport already accepted. The provider layer supplies the finite protocol-local bounds the wire format needs — JSON nesting depth, inspected member count, retained scalar string bytes, isolated usage object bytes, SSE event bytes, and SSE event count — and refuses a bound of zero rather than making parsing ambiguous; it introduces no conflicting global defaults. The observer queue is bounded separately by its own fixed capacity. The retained usage bound never exceeds the `provider_usage.raw_usage_json` column bound, so a usage object the parser accepts is always writable.

Parsers reject or abandon semantic work before allocating from untrusted declared lengths. One document is materialised at most once, with nesting depth, member counts, and duplicate object keys enforced while the value is built. The same rules govern a request body, a non-stream response body, and one SSE event payload, so a duplicate object key is refused at any depth of any of the three. JSON and SSE are bounded for bytes, nesting, items/events, and checked counters. Invalid UTF-8 or a raw NUL byte anywhere in a bounded request or response body yields a malformed observation before any field is read; deeply nested JSON, never-ending events, and excessive event counts yield a resource-limit observation. None of them panic or consume unbounded memory.

The string bound applies to the scalars an observation retains, not to the document carrying them. A retained value that is oversized, wrongly typed, or carries an embedded NUL — including a NUL written as an escape sequence, which valid JSON text may contain — is dropped, its observation is partial, and the remaining allowlisted metadata is still recorded. The document is never rejected for the size of a value the parser does not keep, so a long string that is never retained leaves the observation complete.

## Privacy boundary

The semantic record allowlist is the fields named above. Full request/response headers and bodies are never placed in observations, logs, debug output, IPC records, or events. `Authorization`, cookies, API keys, prompt text, instructions, metadata values, tool arguments, passwords, `user`, `prompt_cache_key`, and `safety_identifier` are excluded by construction rather than scrubbed by blacklist. A forward that failed in transport contributes its failure classification and nothing else: no endpoint, no headers, no body bytes.

Debug implementations expose statuses and counts but not raw provider usage bytes or content. Raw usage may contain only the isolated `usage` object and is bounded before persistence. Those bytes live in the relational row alone; an event reports them only as a byte count. Phase 2 emits no cloud export.

## SQLite migration 0002

`0001_initial.sql` is immutable. `0002_provider_observability.sql` is additive and upgrades existing v1 databases atomically.

Existing relational boundaries remain authoritative:

```text
session
  -> LlmInference operation
    -> provider_request
      -> provider_attempt
        -> provider_usage
```

Migration 0002 extends `provider_requests` with provider/protocol/parser identity, semantic observation status, and the allowlisted request metadata; `provider_attempts` with provider response ID, response model, response state, the creation value the provider reported, incomplete reason, error code, transport-failure classification, semantic observation status, streaming flag, chunk and byte counters, the measured `ttfb_us`, `ttft_us`, and `duration_us` microsecond counters, and bounded anomaly metadata; and `provider_usage` with the exact raw usage bytes, normalized cached/reasoning/total values, normalizer version, and bounded anomaly metadata. Observation timings are elapsed microsecond counters, never timestamps: the only wall-clock columns of an attempt remain the `started_at`/`ended_at` pair 0001 already defines, and 0002 declares no column that an observation cannot populate. Unknown values are nullable. Foreign keys continue to prevent orphan request, attempt, or usage rows. A logical request may have multiple attempts; a retry adds an attempt to the existing request row, and usage is attached to an attempt and is not double-counted.

The attempt lifecycle is derived from every piece of evidence one observation carries, so a failed attempt is never indistinguishable from one still in flight. A transport failure, a non-2xx upstream status, or a provider error code is terminal error evidence whatever the semantic state says; cancellation, disconnection, and incompleteness stay distinct; and `Completed` requires the provider itself to have reported a completed response with no error evidence beside it. A transport failure additionally persists its classification label in `transport_error`.

The single daemon-owned writer remains the only SQLite mutation path. One settled observation commits its provider request row — or, for a retry, only its new attempt row — its attempt row, its usage row when a response was observed, its canonical events, and, once the attempt lifecycle is terminal, the operation state that closes the inference, all in one `WriteBatch`: every command or none. That operation state is `Completed`, `Incomplete`, `Cancelled`, `Errored`, or `Disconnected`; an attempt with no terminal evidence leaves the operation open. Events supplement relational rows; they are not the sole source of truth.

Migration application is transactional. Empty databases apply v1 then v2; v1 databases apply only v2; interrupted migration leaves the previous version visible; future schema versions are rejected. No down migration is executed automatically; rollback means the failed transaction exposes no v2 schema or metadata version.

## Event vocabulary

Canonical event names are:

- `provider.request.observed`
- `provider.response.started`
- `provider.response.completed`
- `provider.response.incomplete`
- `provider.response.failed`
- `provider.usage.observed`
- `provider.usage.normalized`
- `provider.observation.partial`

Every event one observation supports is committed in the same atomic batch as that observation's relational rows. `provider.request.observed` is recorded for every observation. `provider.response.started` is recorded once an upstream status or upstream bytes were observed. Exactly one terminal response event is recorded once the attempt lifecycle is terminal: `provider.response.completed` for a completed response, `provider.response.failed` for a failed one, and `provider.response.incomplete` for every other terminal outcome, whose exact state stays in the relational row. `provider.usage.observed` is recorded when raw usage was retained, `provider.usage.normalized` when usage was normalized, and `provider.observation.partial` when either half of the observation is not `Complete`.

Every payload carries the operation, request, and attempt identity together with the attempt ordinal. Payloads otherwise contain identifiers, versions, states, counts, and timings only. They contain no headers, content, raw usage JSON, or sensitive provider metadata; retained usage appears only as its byte count.

## Runtime and compatibility

Official runtime targets are Linux and macOS. Windows runtime IPC remains unsupported in Phase 2; portable abstractions and compileability are retained where practical.

All Phase 0/1 behavior remains required, especially byte-exact binary forwarding, chunked streaming, 429/500 pass-through, fail-open auxiliary sinks, bounded bodies, crash/restart, SQLite busy handling, and CAS failure behavior. Phase 2 does not relax existing tests or reclassify unknown data as zero.

## Verification contract

Completion requires format, strict Clippy, workspace tests, and workspace build on the documented Rust toolchain. CI runs those gates on Linux and macOS, with optional Windows compile-only. Manual and scheduled bounded fuzz jobs cover SSE framing, request and response JSON parsing, usage normalization, and the stream state machine; each target asserts the invariant it documents rather than only surviving without a panic.

Golden fixtures contain raw request/response bytes plus expected semantic JSON for completed, incomplete, failed, streamed, cancelled, usage-present, usage-missing, echoed-usage, cached-input, reasoning-output, tool-call, malformed, unknown-field/event, and large bounded cases. Streaming fixtures are exercised at fixed and randomized chunk boundaries.

An independent review must specifically inspect hidden reserialization, hot-path blocking, observer backpressure, stream memory growth, unknown-as-zero bugs, cached/reasoning arithmetic, orphan DAG rows, secret leakage, migration atomicity, parser-version ambiguity, cancellation state, incomplete/completed confusion, unbounded JSON/SSE, and retry double counting. Phase 2 is not complete until all P0/P1 findings are resolved.
