ALTER TABLE compression_candidates ADD COLUMN provider_readability TEXT NOT NULL DEFAULT 'unknown'
    CHECK(provider_readability IN ('human_readable_structured', 'provider_compatible_control', 'opaque_custom_encoding', 'unknown'));
ALTER TABLE compression_candidates ADD COLUMN json_root_kind TEXT;
ALTER TABLE compression_candidates ADD COLUMN json_array_length_bucket TEXT;
ALTER TABLE compression_candidates ADD COLUMN json_object_key_count_bucket TEXT;
ALTER TABLE compression_candidates ADD COLUMN json_homogeneity_basis_points INTEGER
    CHECK(json_homogeneity_basis_points BETWEEN 0 AND 10000);
ALTER TABLE compression_candidates ADD COLUMN json_primitive_cell_ratio_basis_points INTEGER
    CHECK(json_primitive_cell_ratio_basis_points BETWEEN 0 AND 10000);
ALTER TABLE compression_candidates ADD COLUMN json_nested_cell_ratio_basis_points INTEGER
    CHECK(json_nested_cell_ratio_basis_points BETWEEN 0 AND 10000);
ALTER TABLE compression_candidates ADD COLUMN text_shape TEXT;

CREATE INDEX compression_candidates_readability_idx
    ON compression_candidates(experiment_id, provider_readability, status);
