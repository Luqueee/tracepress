ALTER TABLE provider_requests ADD COLUMN request_kind TEXT NOT NULL DEFAULT 'turn' CHECK(request_kind IN ('turn', 'compaction'));
