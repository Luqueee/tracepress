ALTER TABLE compression_experiments ADD COLUMN shadow_queue_full_drops INTEGER NOT NULL DEFAULT 0 CHECK(shadow_queue_full_drops >= 0);
ALTER TABLE compression_experiments ADD COLUMN shadow_byte_budget_drops INTEGER NOT NULL DEFAULT 0 CHECK(shadow_byte_budget_drops >= 0);
ALTER TABLE compression_experiments ADD COLUMN shadow_work_budget_drops INTEGER NOT NULL DEFAULT 0 CHECK(shadow_work_budget_drops >= 0);
ALTER TABLE compression_experiments ADD COLUMN shadow_worker_closed_drops INTEGER NOT NULL DEFAULT 0 CHECK(shadow_worker_closed_drops >= 0);
ALTER TABLE compression_experiments ADD COLUMN shadow_persistence_drops INTEGER NOT NULL DEFAULT 0 CHECK(shadow_persistence_drops >= 0);
