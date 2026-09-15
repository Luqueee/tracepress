#!/usr/bin/env python3
"""Contract tests for the Phase 4.6 metadata-only lifetime analyzer."""

from __future__ import annotations

import importlib.util
import sqlite3
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "scripts" / "analyze_toolresult_lifetime.py"
SPEC = importlib.util.spec_from_file_location("toolresult_lifetime", PATH)
assert SPEC and SPEC.loader
ANALYZER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ANALYZER
SPEC.loader.exec_module(ANALYZER)
SHADOW_PATH = ROOT / "scripts" / "run_shadow_eviction.py"
SHADOW_SPEC = importlib.util.spec_from_file_location("shadow_eviction", SHADOW_PATH)
assert SHADOW_SPEC and SHADOW_SPEC.loader
SHADOW = importlib.util.module_from_spec(SHADOW_SPEC)
sys.modules[SHADOW_SPEC.name] = SHADOW
SHADOW_SPEC.loader.exec_module(SHADOW)


def fixture(path: Path) -> None:
    connection = sqlite3.connect(path)
    connection.executescript("""
        CREATE TABLE context_snapshots (
          snapshot_id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
          provider_request_id TEXT, inference_operation_id TEXT NOT NULL,
          status TEXT NOT NULL, started_at_us INTEGER, completed_at_us INTEGER
        );
        CREATE TABLE provider_requests (request_id TEXT PRIMARY KEY, request_kind TEXT);
        CREATE TABLE context_block_occurrences (
          block_occurrence_id TEXT PRIMARY KEY, snapshot_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
          kind TEXT NOT NULL, raw_bytes INTEGER NOT NULL, estimated_tokens INTEGER,
          exact_fingerprint BLOB, semantic_fingerprint BLOB, tool_call_id TEXT, tool_name TEXT,
          detected_kind TEXT, raw_value_start INTEGER
        );
        INSERT INTO provider_requests VALUES ('r1', 'turn'), ('r2', 'turn'), ('r3', 'compaction');
        INSERT INTO context_snapshots VALUES
          ('s1r1', 'session-a', 'r1', 'op1', 'complete', 1, 1),
          ('s1r2', 'session-a', 'r2', 'op2', 'complete', 2, 2),
          ('s1r3', 'session-a', 'r3', 'op3', 'complete', 3, 3),
          ('s2r1', 'session-b', 'r1', 'op4', 'complete', 1, 1);
        INSERT INTO context_block_occurrences VALUES
          ('a1', 's1r1', 1, 'tool_result', 4000, 1000, X'01', X'11', 'call-a', 'search', 'json', 3600),
          ('a2', 's1r2', 1, 'tool_result', 4000, 1000, X'01', X'11', 'call-a', 'search', 'json', 3600),
          ('a3', 's1r3', 1, 'tool_result', 4000, 1000, X'01', X'11', 'call-a', 'search', 'json', 3600),
          ('same-content-new-call', 's1r2', 2, 'tool_result', 4000, 1000, X'01', X'11', 'call-b', 'search', 'json', 3600),
          ('other-session', 's2r1', 1, 'tool_result', 4000, 1000, X'01', X'11', 'call-a', 'search', 'json', 3600),
          ('missing-association', 's1r2', 3, 'tool_result', 800, 200, X'02', X'12', NULL, 'search', 'json', 100),
          ('context', 's1r1', 0, 'text', 400, 100, X'03', X'13', NULL, NULL, 'plain_text', 0),
          ('context2', 's1r2', 0, 'text', 400, 100, X'04', X'14', NULL, NULL, 'plain_text', 0),
          ('context3', 's1r3', 0, 'text', 400, 100, X'05', X'15', NULL, NULL, 'plain_text', 0);
    """)
    connection.commit()
    connection.close()


class ToolResultLifetimeTests(unittest.TestCase):
    def test_lineage_never_joins_equal_content_from_another_call_or_session(self) -> None:
        with TemporaryDirectory() as temporary:
            database = Path(temporary) / "source.sqlite"
            fixture(database)
            rows, snapshots = ANALYZER.load_occurrences(database, {}, "synthetic")
            result = ANALYZER.report(rows, len(snapshots), [database])
        self.assertEqual(result["lineages"]["count"], 3)
        self.assertEqual(result["dataset"]["lineage_coverage_exclusions"]["missing_tool_call_id"], 1)
        self.assertEqual(result["lineages"]["retained_token_exposure"], 2000)
        self.assertEqual(result["compaction_interaction"]["compaction_snapshots"], 1)
        self.assertEqual(result["offline_gate"]["lineages_with_follow_up_observation"], 2)

    def test_policy_charges_stub_and_recovery_schema_costs(self) -> None:
        with TemporaryDirectory() as temporary:
            database = Path(temporary) / "source.sqlite"
            fixture(database)
            rows, snapshots = ANALYZER.load_occurrences(database, {}, "synthetic")
            result = ANALYZER.report(rows, len(snapshots), [database])
        e2 = next(item for item in result["policy_simulations"] if item["policy_id"] == "E2")
        self.assertEqual(e2["eligible_historical_occurrences"], 1)
        self.assertGreater(e2["stub_overhead"], 0)
        self.assertGreater(e2["recovery_tool_schema_overhead"], 0)
        self.assertEqual(e2["shadow_net_eviction_reduction"], e2["gross_eviction_reduction"] - e2["stub_overhead"] - e2["recovery_tool_schema_overhead"])

    def test_shadow_candidates_are_bounded_and_never_forwarding_mutations(self) -> None:
        with TemporaryDirectory() as temporary:
            database = Path(temporary) / "source.sqlite"
            fixture(database)
            rows, _ = ANALYZER.load_occurrences(database, {}, "synthetic")
            candidates, summary = SHADOW.candidate_rows(rows, ["E1", "E2", "E5"], 2)
        self.assertEqual(len(candidates), 2)
        self.assertGreater(summary["candidates_dropped_by_limit"], 0)
        self.assertEqual(summary["forwarding_mutations"], 0)
        self.assertTrue(all(item["policy_id"] in {"E1", "E2", "E5"} for item in candidates))
        self.assertTrue(all("lineage_id" in item and "first_modified_offset" in item for item in candidates))


if __name__ == "__main__":
    unittest.main()
