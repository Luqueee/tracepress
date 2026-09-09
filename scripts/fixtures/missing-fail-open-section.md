# Tracepress Phase 0 and Phase 1 Architecture Contract

Status: pre-code contract. This document is the gate for workspace bootstrap and product implementation.

## Critical review and resolutions

The source plan contains three tensions. First, it names compression and provider adapters broadly, while Phase 1 requires transparent forwarding. Resolution: Phase 0 defines boundaries only, and Phase 1 performs no lossy transformation and no compression. Second, a recovery API is named before lossy output exists. Resolution: Phase 1 stores byte-addressable content and metadata, but exposes no retrieval UX. Third, provider usage and telemetry are described as future data, but their semantics vary and could leak secrets. Resolution: Phase 1 captures bounded transport metadata only, with SQL NULL for unknown values. Usage normalization and telemetry remain deferred.

The sole forwarding seam is OpenAI-compatible `/v1/chat/completions` over HTTP/1.1. Request and response bodies are opaque bytes. Headers are allowlisted metadata, never a persisted provider header dump.

The exact boundary contract says request body bytes are opaque and response body bytes are opaque. The rule is: never persist complete provider headers.

## Architectural invariants

1. Every externally visible session, operation, request, attempt, tool call, decision, recovery, evaluation, and policy assignment ID is UUIDv7. SQLite rowids are internal only.
2. A content ID is SHA-256(raw bytes), computed over the exact bytes received. Encoding, newline conversion, parsing, and redaction never alter the stored raw object.
3. Unknown, unavailable, or inapplicable values are SQL NULL, never zero. This applies to tokens, cost, latency components, cache values, quality, and provider usage.
4. Raw content bytes are immutable. A content object is never edited in place; a new byte sequence has a new content ID.
5. A per-session logical content binding is frozen when selected. Reusing the same logical content in that session returns the same rendered representation and versions. A conflicting replacement is rejected.
6. There is one SQLite writer, owned by `tracepressd`. Clients and hooks use IPC and cannot open a write connection.
The single daemon writer is the only component permitted to write SQLite.
7. Future lossy persistence is one foreign-key-valid atomic transaction: insert raw content objects, then the compression decision, then its recovery mapping, then the frozen binding, then the append-only event. Commit that transaction before transformed bytes become eligible for output. An orphan CAS file is acceptable; a committed dangling reference is not.
8. Phase 1 performs no lossy transformation and no compression. It forwards the original body bytes unchanged.
9. Optimization, auxiliary persistence, policy, telemetry, analytics, or redaction failures fail open: forward the original request whenever forwarding remains possible.
10. A success result is reported only after its promised operation has completed. An interrupted, cancelled, stale, partial, or uncommitted operation cannot be reported as success. This prevents misleading success.
11. All resource limits are explicit configuration values. Bodies, frames, queues, and processing each have finite bounds; inputs over a bound are rejected or bypassed without unbounded allocation.
12. Privacy boundaries are local raw storage versus external metadata only. Authorization, Bearer credentials, API keys, AWS secrets, GitHub tokens, cookies, environment values, absolute paths, prompts, tool output, and source code are redacted or excluded from logs and exports. Never persist complete provider headers.

## Bounded resource contract

The configuration must provide finite `max_request_body_bytes`, `max_response_body_bytes`, `max_ipc_frame_bytes`, `max_ipc_queue_items`, `max_processing_time_ms`, and `max_line_bytes`. It must also provide finite `max_json_nesting` and `max_json_items` for metadata inspection. These are bounded body, bounded frame, bounded queue, and bounded processing contracts, not invented defaults. Startup rejects missing or non-positive limits. Phase 1 treats bodies as opaque and therefore does not parse JSON to enforce semantic limits. An oversized request is deterministically rejected before upstream forwarding with a typed size error. A streaming response exceeding `max_response_body_bytes` is deterministically terminated, marked incomplete, and never reported successful. No path buffers infinite output.

Streaming uses bounded chunks and bounded assembled response storage. On client cancellation or disconnect, the upstream request is cancelled, the attempt is recorded as cancelled or disconnected, and no success is emitted. Malformed SSE-like bytes are forwarded as bytes if transport framing remains valid; malformed transport framing is an explicit error.

## SQLite v1 schema

All tables use handwritten, numbered migrations. Foreign keys are enabled (`foreign_keys=ON`), WAL is enabled, and the connection sets `busy_timeout=5000`. Durability modes are `balanced` with `synchronous=NORMAL` and `strict` with `synchronous=FULL`. The daemon writer serializes transactions. External IDs are `TEXT NOT NULL UNIQUE` UUIDv7 values unless stated otherwise. Byte counts are integers. Unknown values are nullable.

```sql
schema_metadata(schema_version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)
sessions(session_id TEXT PRIMARY KEY, started_at TEXT NOT NULL, ended_at TEXT,
         state TEXT NOT NULL, ingress_key TEXT NOT NULL UNIQUE)
operations(operation_id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions,
           kind TEXT NOT NULL, started_at TEXT NOT NULL, ended_at TEXT, status TEXT NOT NULL)
causal_edges(parent_operation_id TEXT NOT NULL REFERENCES operations,
             child_operation_id TEXT NOT NULL REFERENCES operations,
             relationship TEXT NOT NULL, PRIMARY KEY(parent_operation_id, child_operation_id, relationship),
             CHECK(parent_operation_id <> child_operation_id))
provider_requests(request_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL REFERENCES operations,
                  route TEXT NOT NULL, method TEXT NOT NULL, request_bytes INTEGER NOT NULL)
provider_attempts(attempt_id TEXT PRIMARY KEY, request_id TEXT NOT NULL REFERENCES provider_requests,
                  ordinal INTEGER NOT NULL, status_code INTEGER, started_at TEXT NOT NULL,
                  ended_at TEXT, status TEXT NOT NULL, UNIQUE(request_id, ordinal))
provider_usage(attempt_id TEXT PRIMARY KEY REFERENCES provider_attempts,
               input_total INTEGER, input_uncached INTEGER, cache_read INTEGER, cache_write INTEGER,
               output_total INTEGER, reasoning INTEGER, usage_status TEXT)
content_objects(content_id TEXT PRIMARY KEY, raw_bytes BLOB, external_ref TEXT UNIQUE,
                byte_length INTEGER NOT NULL, content_kind TEXT NOT NULL, pin_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL, CHECK((raw_bytes IS NULL) <> (external_ref IS NULL)))
content_occurrences(occurrence_id TEXT PRIMARY KEY, content_id TEXT NOT NULL REFERENCES content_objects,
                    session_id TEXT NOT NULL REFERENCES sessions, operation_id TEXT REFERENCES operations,
                    role TEXT NOT NULL, observed_at TEXT NOT NULL)
content_bindings(binding_id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions,
                 logical_content_id TEXT NOT NULL, raw_content_id TEXT NOT NULL REFERENCES content_objects,
                 rendered_content_id TEXT NOT NULL REFERENCES content_objects, compressor_version TEXT NOT NULL,
                 policy_version TEXT NOT NULL, frozen INTEGER NOT NULL CHECK(frozen=1),
                 UNIQUE(session_id, logical_content_id))
compression_decisions(decision_id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions,
                      operation_id TEXT NOT NULL REFERENCES operations, input_content_id TEXT NOT NULL REFERENCES content_objects,
                      output_content_id TEXT NOT NULL REFERENCES content_objects, fidelity TEXT NOT NULL,
                      recoverable INTEGER NOT NULL, compressor TEXT NOT NULL, compressor_version TEXT NOT NULL,
                      policy_version TEXT NOT NULL, raw_bytes INTEGER NOT NULL, output_bytes INTEGER NOT NULL,
                      estimated_raw_tokens INTEGER, estimated_output_tokens INTEGER, target_tokens INTEGER,
                      latency_us INTEGER, feature_schema_version TEXT NOT NULL, features BLOB)
recoveries(recovery_id TEXT PRIMARY KEY, decision_id TEXT NOT NULL UNIQUE REFERENCES compression_decisions,
           raw_content_id TEXT NOT NULL REFERENCES content_objects, provenance_content_id TEXT REFERENCES content_objects)
policy_assignments(policy_assignment_id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions,
                   policy_version TEXT NOT NULL, assigned_at TEXT NOT NULL, chosen_action TEXT NOT NULL,
                   candidate_actions BLOB, action_probability REAL, random_seed INTEGER, feature_vector BLOB)
evaluations(evaluation_id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions,
            operation_id TEXT REFERENCES operations, provider_cost_microusd INTEGER, input_tokens INTEGER,
            cache_tokens INTEGER, output_tokens INTEGER, recoveries INTEGER, reruns INTEGER,
            latency_ms INTEGER, task_success INTEGER, user_correction INTEGER, quality_score REAL)
events(seq INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE,
       session_id TEXT REFERENCES sessions, operation_id TEXT REFERENCES operations,
       timestamp TEXT NOT NULL, event_type TEXT NOT NULL, payload BLOB NOT NULL, schema_version TEXT NOT NULL)
```

Relational rows are the query model. `events` is append-only: rows are inserted in sequence, never updated or deleted, and each event has one immutable payload. Critical rows and the corresponding event are committed in one writer transaction where possible. A failed transaction emits no logical success event. Orphan CAS files are acceptable and garbage collection may remove only unpinned, unreferenced files after the retention rule permits it.

The future lossy transaction is a foreign-key-valid atomic transaction: raw object references, decision, recovery mapping, binding, and event are committed before output eligibility. A stale session is rejected, and an uncertain transaction cannot produce misleading success.

## Principal Rust traits

Traits describe capabilities, not transports or concrete storage. Method error types are typed and cancellation is explicit in async methods.

```rust
trait IpcTransport {
    type Peer;
    async fn accept(&self) -> Result<Self::Peer, IpcError>;
    async fn connect(&self, endpoint: &Endpoint) -> Result<Self::Peer, IpcError>;
}

trait BlobStore {
    async fn put(&self, bytes: &[u8]) -> Result<ContentId, BlobError>;
    async fn get(&self, id: ContentId) -> Result<RawBytes, BlobError>;
    async fn exists(&self, id: ContentId) -> Result<bool, BlobError>;
    async fn delete(&self, id: ContentId) -> Result<(), BlobError>;
}

trait StorageWriter {
    async fn commit_critical(&self, command: CriticalCommand) -> Result<CommitReceipt, StorageError>;
    async fn append_event(&self, event: Event) -> Result<EventReceipt, StorageError>;
}

trait ProviderForwarder {
    async fn forward(&self, request: OpaqueHttpRequest) -> Result<OpaqueHttpResponse, ForwardError>;
}

trait SessionRegistry {
    async fn create(&self, request: SessionRequest) -> Result<Session, SessionError>;
    async fn close(&self, id: SessionId, reason: CloseReason) -> Result<(), SessionError>;
}
```

The wire protocol is framed and typed independently of Unix sockets. Unix domain sockets are first on Unix, named pipes are the Windows implementation, and authenticated localhost TCP is an explicit fallback. The protocol carries bounded frames and never logs authentication material.

## Exact request flow

1. `tracepress run <agent>` asks the daemon for a session. The daemon allocates a UUIDv7 `session_id`, an isolated ingress key, and finite limits.
2. The agent sends an authenticated IPC request to the session ingress. The daemon validates framing and route metadata at the boundary, then treats the HTTP body as opaque bytes.
3. The proxy records request metadata and a `provider_request` operation through the single daemon writer. It does not rewrite, decode, normalize, compress, or redact the forwarded body.
4. The provider forwarder sends the same HTTP/1.1 method, route, and body bytes to the upstream OpenAI-compatible endpoint. Only an allowlisted, redacted metadata subset is eligible for persistence.
5. Upstream status, bounded transport metadata, and response body bytes return through the proxy. Chunked and SSE-like payload bytes remain unchanged. Status 429 and 500 pass through unchanged.
6. The daemon records completion, incomplete, cancellation, disconnect, or error state. Phase 1 does not capture provider usage and stores no usage; parsing and normalization begin in Phase 2. It never invents zero usage or a successful outcome, and it performs no semantic response inspection.
7. The response body bytes are returned to the agent exactly as received. If auxiliary storage, event logging, policy lookup, or telemetry fails, the original forwarding path continues.
The request body bytes are opaque and the response body bytes are opaque; Phase 1 has no provider usage capture and no semantic response inspection. An oversized request is deterministically rejected before upstream forwarding. A streaming response that exceeds its limit is deterministically terminated and marked incomplete.
The OpenAI-compatible `/v1/chat/completions` request flow is transport-only: no Phase 1 provider usage capture, no semantic response inspection, deterministically rejects an oversized request, and deterministically terminates an oversized response as incomplete.

## Crash consistency contracts

These are later exercised with subprocess interruption and real temporary SQLite and filesystem state. Each scenario has an objective result.

The future lossy transaction is a foreign-key-valid atomic transaction: raw object references, decision, recovery mapping, binding, and event are committed before output eligibility. A stale session is rejected, and an uncertain transaction cannot produce misleading success.

| Scenario | Pass contract | Fail contract |
| --- | --- | --- |
| Crash during a critical transaction | Reopen shows either the prior committed state or the complete next state. | No partial logical row set or dangling reference. |
| Interrupted migration | Reopen applies the migration atomically or resumes from the prior version. | Schema metadata must not claim an unapplied version. |
| CAS write or disk failure | Original bytes remain forwardable and no DB reference points to missing content. | Lossy output is never emitted. |
| Power loss model | Every committed reference resolves to exact bytes after reopen. | A missing reference or altered raw byte fails the test. |
| SQLite busy | Bounded retry or typed failure returns control within the configured bound. | The request is not falsely reported successful. |
| GC and recovery race | Pinned or recoverable content survives. | GC must not delete a blob needed by recovery. |
| Cancellation or interruption | Attempt is marked cancelled, incomplete, or disconnected. | No completed status is emitted for a partial response. |
| Stale state | Stale session or binding is rejected with a typed state error. | Stale state cannot overwrite a frozen binding. |
| Misleading success | Success is emitted only after commit and complete forwarding. | Any uncertain outcome is explicit, never success. |


## Privacy and redaction boundary

Raw bytes remain local and immutable. Logs and future external exports contain metadata only, after redaction. Authorization and Bearer values, API keys, AWS secrets, GitHub tokens, OpenAI keys, Anthropic keys, cookies, environment values, absolute paths, prompts, tool outputs, and user source code are never exported. Never persist complete provider headers. Content IDs are SHA-256(raw bytes) internally; any future external identifier uses HMAC-SHA-256 with a local secret, not a deterministic raw-content hash.

## Scope exclusions

Phase 0 defines architecture, schema, traits, flow, and failure contracts only. Phase 1 includes the daemon, portable IPC boundary, migrations, hybrid byte store, session lifecycle, operation DAG, and transparent proxy. Phase 1 has no lossy transformation and no compression. Provider usage normalization, provider-specific adapters, OTel, Parquet, DuckDB, ML, adaptive policy, A/A, A/B, shadow optimization, recovery UX, and dataset export are explicitly deferred. No production Rust modules are part of this contract.
