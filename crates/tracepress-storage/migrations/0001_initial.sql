CREATE TABLE schema_metadata (
    schema_version INTEGER NOT NULL PRIMARY KEY,
    applied_at TEXT NOT NULL
) STRICT;

CREATE TABLE sessions (
    session_id TEXT NOT NULL PRIMARY KEY,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    state TEXT NOT NULL,
    ingress_key TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE operations (
    operation_id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    kind TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    status TEXT NOT NULL
) STRICT;

CREATE TABLE causal_edges (
    parent_operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    child_operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    relationship TEXT NOT NULL,
    PRIMARY KEY(parent_operation_id, child_operation_id, relationship),
    CHECK(parent_operation_id <> child_operation_id)
) STRICT;

CREATE TABLE provider_requests (
    request_id TEXT NOT NULL PRIMARY KEY,
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    route TEXT NOT NULL,
    method TEXT NOT NULL,
    request_bytes INTEGER NOT NULL CHECK(request_bytes >= 0)
) STRICT;

CREATE TABLE provider_attempts (
    attempt_id TEXT NOT NULL PRIMARY KEY,
    request_id TEXT NOT NULL REFERENCES provider_requests(request_id),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    status_code INTEGER CHECK(status_code BETWEEN 100 AND 599),
    started_at TEXT NOT NULL,
    ended_at TEXT,
    status TEXT NOT NULL,
    UNIQUE(request_id, ordinal)
) STRICT;

CREATE TABLE provider_usage (
    attempt_id TEXT NOT NULL PRIMARY KEY REFERENCES provider_attempts(attempt_id),
    input_total INTEGER CHECK(input_total >= 0),
    input_uncached INTEGER CHECK(input_uncached >= 0),
    cache_read INTEGER CHECK(cache_read >= 0),
    cache_write INTEGER CHECK(cache_write >= 0),
    output_total INTEGER CHECK(output_total >= 0),
    reasoning INTEGER CHECK(reasoning >= 0),
    usage_status TEXT
) STRICT;

CREATE TABLE content_objects (
    content_id TEXT NOT NULL PRIMARY KEY,
    raw_bytes BLOB,
    external_ref TEXT UNIQUE,
    byte_length INTEGER NOT NULL CHECK(byte_length >= 0),
    content_kind TEXT NOT NULL,
    pin_count INTEGER NOT NULL DEFAULT 0 CHECK(pin_count >= 0),
    created_at TEXT NOT NULL,
    CHECK((raw_bytes IS NULL) <> (external_ref IS NULL))
) STRICT;

CREATE TABLE content_occurrences (
    occurrence_id TEXT NOT NULL PRIMARY KEY,
    content_id TEXT NOT NULL REFERENCES content_objects(content_id),
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    operation_id TEXT REFERENCES operations(operation_id),
    role TEXT NOT NULL,
    observed_at TEXT NOT NULL
) STRICT;

CREATE TABLE content_bindings (
    binding_id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    logical_content_id TEXT NOT NULL,
    raw_content_id TEXT NOT NULL REFERENCES content_objects(content_id),
    rendered_content_id TEXT NOT NULL REFERENCES content_objects(content_id),
    compressor_version TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    frozen INTEGER NOT NULL CHECK(frozen = 1),
    UNIQUE(session_id, logical_content_id)
) STRICT;

CREATE TABLE compression_decisions (
    decision_id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    input_content_id TEXT NOT NULL REFERENCES content_objects(content_id),
    output_content_id TEXT NOT NULL REFERENCES content_objects(content_id),
    fidelity TEXT NOT NULL,
    recoverable INTEGER NOT NULL CHECK(recoverable IN (0, 1)),
    compressor TEXT NOT NULL,
    compressor_version TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    raw_bytes INTEGER NOT NULL CHECK(raw_bytes >= 0),
    output_bytes INTEGER NOT NULL CHECK(output_bytes >= 0),
    estimated_raw_tokens INTEGER CHECK(estimated_raw_tokens >= 0),
    estimated_output_tokens INTEGER CHECK(estimated_output_tokens >= 0),
    target_tokens INTEGER CHECK(target_tokens >= 0),
    latency_us INTEGER CHECK(latency_us >= 0),
    feature_schema_version TEXT NOT NULL,
    features BLOB
) STRICT;

CREATE TABLE recoveries (
    recovery_id TEXT NOT NULL PRIMARY KEY,
    decision_id TEXT NOT NULL UNIQUE REFERENCES compression_decisions(decision_id),
    raw_content_id TEXT NOT NULL REFERENCES content_objects(content_id),
    provenance_content_id TEXT REFERENCES content_objects(content_id)
) STRICT;

CREATE TABLE policy_assignments (
    policy_assignment_id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    policy_version TEXT NOT NULL,
    assigned_at TEXT NOT NULL,
    chosen_action TEXT NOT NULL,
    candidate_actions BLOB,
    action_probability REAL CHECK(action_probability BETWEEN 0.0 AND 1.0),
    random_seed INTEGER CHECK(random_seed >= 0),
    feature_vector BLOB
) STRICT;

CREATE TABLE evaluations (
    evaluation_id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    operation_id TEXT REFERENCES operations(operation_id),
    provider_cost_microusd INTEGER CHECK(provider_cost_microusd >= 0),
    input_tokens INTEGER CHECK(input_tokens >= 0),
    cache_tokens INTEGER CHECK(cache_tokens >= 0),
    output_tokens INTEGER CHECK(output_tokens >= 0),
    recoveries INTEGER CHECK(recoveries >= 0),
    reruns INTEGER CHECK(reruns >= 0),
    latency_ms INTEGER CHECK(latency_ms >= 0),
    task_success INTEGER CHECK(task_success IN (0, 1)),
    user_correction INTEGER CHECK(user_correction IN (0, 1)),
    quality_score REAL
) STRICT;

CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    session_id TEXT REFERENCES sessions(session_id),
    operation_id TEXT REFERENCES operations(operation_id),
    timestamp TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload BLOB NOT NULL,
    schema_version TEXT NOT NULL
) STRICT;

CREATE TRIGGER events_prevent_update
BEFORE UPDATE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

CREATE TRIGGER events_prevent_delete
BEFORE DELETE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;
