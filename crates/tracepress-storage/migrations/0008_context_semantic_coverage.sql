ALTER TABLE context_analysis_metrics ADD COLUMN unknown_block_count INTEGER
    CHECK(unknown_block_count >= 0);

ALTER TABLE context_analysis_metrics ADD COLUMN semantic_coverage_basis_points INTEGER
    CHECK(semantic_coverage_basis_points BETWEEN 0 AND 10000);
