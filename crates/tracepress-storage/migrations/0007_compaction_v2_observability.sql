-- Keep the released v5 column intact as a compatibility record while introducing the
-- canonical request-kind vocabulary. Renaming preserves the old CHECK constraint and avoids
-- rebuilding a table that has live provider-attempt foreign keys.
ALTER TABLE provider_requests RENAME COLUMN request_kind TO legacy_request_kind;
ALTER TABLE provider_requests ADD COLUMN request_kind TEXT NOT NULL DEFAULT 'turn';
UPDATE provider_requests
SET request_kind = CASE legacy_request_kind
    WHEN 'compaction' THEN 'compaction_legacy'
    ELSE legacy_request_kind
END;
ALTER TABLE provider_requests ADD COLUMN compaction_trigger TEXT
    CHECK(compaction_trigger IN ('manual', 'auto', 'unknown'));

ALTER TABLE provider_attempts ADD COLUMN compaction_output_seen INTEGER
    CHECK(compaction_output_seen IN (0, 1));
