#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("analyze_shadow_compression.py")
SPEC = importlib.util.spec_from_file_location("shadow_analysis", SCRIPT)
assert SPEC and SPEC.loader
shadow_analysis = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(shadow_analysis)


class ShadowCompressionAnalysisTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.db = Path(self.tempdir.name) / "fixture.sqlite"
        connection = sqlite3.connect(self.db)
        connection.executescript(
            """
            CREATE TABLE compression_experiments (
              experiment_id TEXT PRIMARY KEY, compressor_set_json TEXT, runtime_sha TEXT,
              limits_json TEXT, status TEXT, started_at TEXT, completed_at TEXT,
              forwarding_mutations INTEGER, shadow_drops INTEGER,
              recovery_failures INTEGER, determinism_failures INTEGER);
            CREATE TABLE context_snapshots (snapshot_id TEXT PRIMARY KEY, session_id TEXT);
            CREATE TABLE compression_candidates (
              candidate_id TEXT PRIMARY KEY, experiment_id TEXT, snapshot_id TEXT,
              compressor_id TEXT, compressor_version TEXT, status TEXT,
              original_fingerprint BLOB, cache_risk TEXT);
            CREATE TABLE compression_candidate_metrics (
              candidate_id TEXT PRIMARY KEY, input_bytes INTEGER, output_bytes INTEGER,
              bytes_delta INTEGER, input_estimated_tokens INTEGER,
              output_estimated_tokens INTEGER, estimated_token_delta INTEGER,
              processing_us INTEGER, reversible INTEGER, recovery_verified INTEGER,
              deterministic INTEGER, preserved_prefix_ratio_basis_points INTEGER);
            INSERT INTO compression_experiments VALUES
              ('pilot', '["json.minify"]', 'abc', '{}', 'completed', 'now', 'later', 0, 0, 0, 0);
            INSERT INTO context_snapshots VALUES ('snap-1', 'session-1');
            INSERT INTO compression_candidates VALUES
              ('c1', 'pilot', 'snap-1', 'json.minify', '1', 'applicable', zeroblob(32), 'low'),
              ('c2', 'pilot', 'snap-1', 'json.minify', '1', 'not_applicable', randomblob(32), 'unknown');
            INSERT INTO compression_candidate_metrics VALUES
              ('c1', 100, 60, 40, 25, 15, 10, 90, 1, 1, 1, 8000),
              ('c2', 20, NULL, NULL, 5, NULL, NULL, 10, 1, 0, 0, NULL);
            """
        )
        connection.commit()
        connection.close()

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def test_pending_gates_block_recommendation_and_preserve_nulls(self) -> None:
        with shadow_analysis.readonly_connection(self.db) as connection:
            report = shadow_analysis.build_report(connection, "pilot", {})
        compressor = report["compressors"][0]
        self.assertEqual(compressor["applicability"], 0.5)
        self.assertEqual(compressor["recovery_success_rate"], 1.0)
        self.assertEqual(compressor["estimated_token_reduction_ratio"], 0.4)
        self.assertIsNone(report["recommended_active_candidate"])
        self.assertEqual(report["phase_4_2"], "blocked")

    def test_all_gates_select_exactly_one_non_control_candidate(self) -> None:
        manifest = {
            "quality_gates": {gate: "passed" for gate in shadow_analysis.REQUIRED_GATES},
            "session_workloads": {"session-1": "bug_fix"},
            "recommended_active_candidate": "json.minify",
            "recommendation_rationale": "best measured trade-off in the completed cohort",
        }
        with shadow_analysis.readonly_connection(self.db) as connection:
            report = shadow_analysis.build_report(connection, "pilot", manifest)
        self.assertEqual(report["recommended_active_candidate"], "json.minify")
        self.assertEqual(report["phase_4_2"], "ready_for_design")
        self.assertTrue(report["workload_distribution"]["available"])

    def test_all_gates_do_not_cause_an_automatic_candidate_claim(self) -> None:
        manifest = {
            "quality_gates": {gate: "passed" for gate in shadow_analysis.REQUIRED_GATES}
        }
        with shadow_analysis.readonly_connection(self.db) as connection:
            report = shadow_analysis.build_report(connection, "pilot", manifest)
        self.assertIsNone(report["recommended_active_candidate"])
        self.assertEqual(
            report["recommendation_status"], "blocked_candidate_decision_not_recorded"
        )

    def test_connection_is_enforced_read_only(self) -> None:
        with shadow_analysis.readonly_connection(self.db) as connection:
            with self.assertRaises(sqlite3.OperationalError):
                connection.execute("DELETE FROM compression_experiments")

    def test_explicit_session_scope_excludes_failed_attempts(self) -> None:
        with shadow_analysis.readonly_connection(self.db) as connection:
            report = shadow_analysis.build_report(
                connection, "pilot", {"included_session_ids": []}
            )
        self.assertEqual(report["scope"]["session_count"], 0)
        self.assertEqual(report["scope"]["candidate_count"], 0)
        self.assertEqual(report["compressors"], [])

    def test_nullable_historical_processing_time_does_not_break_report(self) -> None:
        connection = sqlite3.connect(self.db)
        connection.execute(
            "UPDATE compression_candidate_metrics SET processing_us = NULL WHERE candidate_id = 'c2'"
        )
        connection.commit()
        connection.close()
        with shadow_analysis.readonly_connection(self.db) as connection:
            report = shadow_analysis.build_report(connection, "pilot", {})
        self.assertEqual(report["compressors"][0]["processing_us"]["p50"], 90.0)


if __name__ == "__main__":
    unittest.main()
