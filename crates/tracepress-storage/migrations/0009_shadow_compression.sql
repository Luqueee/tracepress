CREATE TABLE compression_experiments (
    experiment_id TEXT NOT NULL PRIMARY KEY CHECK(length(experiment_id) BETWEEN 1 AND 128),
    compressor_set_json TEXT NOT NULL CHECK(json_valid(compressor_set_json)),
    runtime_sha TEXT CHECK(length(runtime_sha) <= 64),
    limits_json TEXT NOT NULL CHECK(json_valid(limits_json)),
    status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'failed')),
    started_at TEXT NOT NULL,
    completed_at TEXT,
    forwarding_mutations INTEGER NOT NULL DEFAULT 0 CHECK(forwarding_mutations >= 0),
    shadow_drops INTEGER NOT NULL DEFAULT 0 CHECK(shadow_drops >= 0),
    recovery_failures INTEGER NOT NULL DEFAULT 0 CHECK(recovery_failures >= 0),
    determinism_failures INTEGER NOT NULL DEFAULT 0 CHECK(determinism_failures >= 0)
) STRICT;

CREATE TABLE compression_candidates (
    candidate_id TEXT NOT NULL PRIMARY KEY,
    experiment_id TEXT NOT NULL REFERENCES compression_experiments(experiment_id),
    snapshot_id TEXT NOT NULL REFERENCES context_snapshots(snapshot_id),
    block_occurrence_id TEXT NOT NULL REFERENCES context_block_occurrences(block_occurrence_id),
    compressor_id TEXT NOT NULL CHECK(length(compressor_id) BETWEEN 1 AND 64),
    compressor_version TEXT NOT NULL CHECK(length(compressor_version) BETWEEN 1 AND 32),
    status TEXT NOT NULL CHECK(status IN (
        'applicable', 'not_applicable', 'no_improvement', 'resource_limit',
        'invalid_input', 'recovery_failed', 'internal_error'
    )),
    original_fingerprint BLOB NOT NULL CHECK(length(original_fingerprint) = 32),
    candidate_fingerprint BLOB CHECK(length(candidate_fingerprint) = 32),
    first_modified_offset INTEGER CHECK(first_modified_offset >= 0),
    preserved_prefix_bytes INTEGER CHECK(preserved_prefix_bytes >= 0),
    cache_risk TEXT NOT NULL CHECK(cache_risk IN ('low', 'medium', 'high', 'unknown')),
    UNIQUE(experiment_id, snapshot_id, block_occurrence_id, compressor_id, compressor_version)
) STRICT;

CREATE INDEX compression_candidates_experiment_idx
    ON compression_candidates(experiment_id, compressor_id, status);
CREATE INDEX compression_candidates_snapshot_idx
    ON compression_candidates(snapshot_id, block_occurrence_id);

CREATE TABLE compression_candidate_metrics (
    candidate_id TEXT NOT NULL PRIMARY KEY REFERENCES compression_candidates(candidate_id),
    input_bytes INTEGER NOT NULL CHECK(input_bytes >= 0),
    output_bytes INTEGER CHECK(output_bytes >= 0),
    bytes_delta INTEGER CHECK(bytes_delta >= 0),
    input_estimated_tokens INTEGER CHECK(input_estimated_tokens >= 0),
    output_estimated_tokens INTEGER CHECK(output_estimated_tokens >= 0),
    estimated_token_delta INTEGER CHECK(estimated_token_delta >= 0),
    processing_us INTEGER NOT NULL CHECK(processing_us >= 0),
    reversible INTEGER NOT NULL CHECK(reversible IN (0, 1)),
    recovery_verified INTEGER NOT NULL CHECK(recovery_verified IN (0, 1)),
    deterministic INTEGER NOT NULL CHECK(deterministic IN (0, 1)),
    preserved_prefix_ratio_basis_points INTEGER
        CHECK(preserved_prefix_ratio_basis_points BETWEEN 0 AND 10000)
) STRICT;

CREATE TABLE compression_recoveries (
    candidate_id TEXT NOT NULL PRIMARY KEY REFERENCES compression_candidates(candidate_id),
    verified INTEGER NOT NULL CHECK(verified IN (0, 1)),
    recovered_fingerprint BLOB CHECK(length(recovered_fingerprint) = 32),
    verified_at_us INTEGER CHECK(verified_at_us >= 0)
) STRICT;
