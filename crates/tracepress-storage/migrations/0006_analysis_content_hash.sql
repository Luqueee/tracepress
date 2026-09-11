ALTER TABLE context_snapshots ADD COLUMN analysis_content_hash BLOB CHECK(length(analysis_content_hash) = 32);
