ALTER TABLE compression_experiments ADD COLUMN shadow_jobs_admitted INTEGER NOT NULL DEFAULT 0 CHECK(shadow_jobs_admitted >= 0);
ALTER TABLE compression_experiments ADD COLUMN shadow_jobs_processed INTEGER NOT NULL DEFAULT 0 CHECK(shadow_jobs_processed >= 0);
ALTER TABLE compression_experiments ADD COLUMN shadow_job_drops INTEGER NOT NULL DEFAULT 0 CHECK(shadow_job_drops >= 0);
ALTER TABLE compression_experiments ADD COLUMN candidate_evaluations_attempted INTEGER NOT NULL DEFAULT 0 CHECK(candidate_evaluations_attempted >= 0);
ALTER TABLE compression_experiments ADD COLUMN candidate_evaluations_completed INTEGER NOT NULL DEFAULT 0 CHECK(candidate_evaluations_completed >= 0);
ALTER TABLE compression_experiments ADD COLUMN candidate_evaluation_drops INTEGER NOT NULL DEFAULT 0 CHECK(candidate_evaluation_drops >= 0);
