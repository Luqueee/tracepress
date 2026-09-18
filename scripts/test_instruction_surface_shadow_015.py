#!/usr/bin/env python3
"""Contract tests for the Phase 6.2 Instruction Surface Shadow pilot."""

from __future__ import annotations

from contextlib import closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("run_instruction_surface_shadow_015.py")
SPEC = importlib.util.spec_from_file_location("instruction_surface_shadow_015", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def create_fixture(path: Path, *, incomplete: bool = False) -> None:
    with closing(sqlite3.connect(path)) as connection:
        connection.executescript(
            """
            CREATE TABLE context_snapshots(
                snapshot_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                provider_request_id TEXT NOT NULL,
                analysis_version INTEGER NOT NULL,
                status TEXT NOT NULL,
                explicit_request_complete INTEGER NOT NULL,
                logical_context_status TEXT NOT NULL,
                correlation_status TEXT NOT NULL
            );
            CREATE TABLE context_block_occurrences(
                snapshot_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                role TEXT,
                origin TEXT NOT NULL,
                raw_bytes INTEGER NOT NULL,
                estimated_tokens INTEGER,
                exact_fingerprint BLOB
            );
            CREATE TABLE provider_attempts(
                attempt_id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL
            );
            CREATE TABLE provider_usage(
                attempt_id TEXT PRIMARY KEY,
                input_total INTEGER,
                input_cached INTEGER,
                input_uncached INTEGER,
                output_total INTEGER,
                output_reasoning INTEGER
            );
            CREATE TABLE operations(
                operation_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL
            );
            CREATE TABLE provider_requests(
                request_id TEXT PRIMARY KEY,
                operation_id TEXT NOT NULL
            );
            """
        )
        snapshots = [
            (
                "snap-1",
                "session-1",
                "request-1",
                1,
                "complete",
                1,
                "explicit_only",
                "correlated",
            ),
            (
                "snap-2",
                "session-1",
                "request-2",
                1,
                "complete",
                1,
                "provider_managed_partial",
                "correlated",
            ),
            (
                "snap-3",
                "session-2",
                "request-3",
                1,
                "complete",
                1,
                "explicit_only",
                "correlated",
            ),
        ]
        if incomplete:
            snapshots[2] = (
                "snap-3",
                "session-2",
                "request-3",
                1,
                "partial",
                0,
                "partial",
                "correlated",
            )
        connection.executemany(
            "INSERT INTO context_snapshots VALUES (?, ?, ?, ?, ?, ?, ?, ?)", snapshots
        )
        blocks = [
            ("snap-1", "text", "developer", "human_authored", 400, 100, b"A"),
            ("snap-1", "text", "developer", "human_authored", 200, 50, b"B"),
            ("snap-2", "text", "developer", "human_authored", 400, 100, b"A"),
            ("snap-3", "text", "developer", "human_authored", 400, 100, b"A"),
            ("snap-3", "text", "developer", "human_authored", 200, 50, b"B"),
            ("snap-1", "unknown", "developer", "unknown", 1_000, None, b"U1"),
            ("snap-2", "unknown", "developer", "unknown", 1_000, None, b"U2"),
            ("snap-3", "unknown", "developer", "unknown", 1_000, None, b"U3"),
        ]
        connection.executemany(
            "INSERT INTO context_block_occurrences VALUES (?, ?, ?, ?, ?, ?, ?)", blocks
        )
        for index in range(1, 4):
            session = "session-1" if index < 3 else "session-2"
            connection.execute(
                "INSERT OR IGNORE INTO operations VALUES (?, ?)",
                (f"operation-{session}", session),
            )
            connection.execute(
                "INSERT INTO provider_requests VALUES (?, ?)",
                (f"request-{index}", f"operation-{session}"),
            )
            connection.execute(
                "INSERT INTO provider_attempts VALUES (?, ?, ?)",
                (f"attempt-{index}", f"request-{index}", 0),
            )
            connection.execute(
                "INSERT INTO provider_usage VALUES (?, ?, ?, ?, ?, ?)",
                (f"attempt-{index}", 1_000, 800, 200, 50, 20),
            )
        connection.commit()


def object_keys(value: object) -> set[str]:
    if isinstance(value, dict):
        return set(value).union(*(object_keys(item) for item in value.values()))
    if isinstance(value, list):
        return set().union(*(object_keys(item) for item in value))
    return set()


class InstructionSurfaceShadowContractTests(unittest.TestCase):
    def test_provider_request_without_snapshot_makes_surface_unavailable(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            database = Path(raw) / "fixture.sqlite3"
            create_fixture(database)
            with closing(sqlite3.connect(database)) as connection:
                connection.execute(
                    "INSERT INTO provider_requests VALUES (?, ?)",
                    ("request-4", "operation-session-1"),
                )
                connection.commit()

            surface = MODULE.characterize_database(database)

        self.assertEqual(surface["requests_observed"], 4)
        self.assertEqual(surface["complete_observations"], 3)
        self.assertEqual(surface["observation_coverage_basis_points"], 7_500)
        self.assertIsNone(surface["developer_text_estimated_tokens"]["value"])
        self.assertIsNone(surface["cross_session_recurring_tokens"]["value"])

    def test_persisted_report_matches_the_aggregate_privacy_contract(self) -> None:
        report_path = (
            SCRIPT.parents[1]
            / "reports/instruction-surface-shadow-015/TRACEPRESS_INSTRUCTION_SURFACE_SHADOW_PILOT_015.json"
        )
        report = json.loads(report_path.read_text(encoding="utf-8"))

        self.assertEqual(report["instruction_surface"]["requests_observed"], 21)
        self.assertEqual(
            report["instruction_surface"]["cross_session_recurring_tokens"]["value"],
            451_710,
        )
        self.assertEqual(
            report["shadow_gate"]["decision"],
            "source_attribution_required_before_policy_design",
        )
        keys = object_keys(report)
        for forbidden in (
            "exact_fingerprint",
            "semantic_path",
            "session_id",
            "request_id",
            "prompt",
            "instruction_text",
            "command_output",
        ):
            self.assertNotIn(forbidden, keys)

    def test_observed_text_without_cross_session_repetition_is_not_reported_as_absent(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            database = Path(raw) / "fixture.sqlite3"
            create_fixture(database)
            surface = MODULE.characterize_database(database)
        surface["cross_session_recurring_tokens"] = {
            "value": 0,
            "source": "locally_estimated",
        }

        report = MODULE.build_report(
            {
                "execution": {
                    "sessions_completed": 2,
                    "sessions_return_code_zero": 2,
                    "sessions_return_code_nonzero": 0,
                    "sessions_timed_out": 0,
                }
            },
            surface,
            sessions_requested=2,
            codex_version_value="codex-cli test",
        )

        self.assertEqual(
            report["shadow_gate"]["decision"],
            "cross_session_repetition_not_observed",
        )

    def test_incomplete_context_analysis_cannot_publish_instruction_totals(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            database = Path(raw) / "fixture.sqlite3"
            create_fixture(database, incomplete=True)

            surface = MODULE.characterize_database(database)
            report = MODULE.build_report(
                {
                    "execution": {
                        "sessions_completed": 2,
                        "sessions_return_code_zero": 2,
                        "sessions_return_code_nonzero": 0,
                        "sessions_timed_out": 0,
                    }
                },
                surface,
                sessions_requested=2,
                codex_version_value="codex-cli test",
            )

        self.assertEqual(surface["observation_coverage_basis_points"], 6_667)
        self.assertIsNone(surface["developer_text_blocks"])
        self.assertIsNone(surface["developer_text_bytes"]["value"])
        self.assertIsNone(surface["developer_text_estimated_tokens"]["value"])
        self.assertIsNone(surface["cross_session_recurring_tokens"]["value"])
        self.assertEqual(report["shadow_gate"]["decision"], "insufficient_shadow_evidence")

    def test_same_request_duplicates_do_not_become_cross_request_repeat_exposure(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            database = Path(raw) / "fixture.sqlite3"
            create_fixture(database)
            with closing(sqlite3.connect(database)) as connection:
                connection.executemany(
                    "INSERT INTO context_block_occurrences VALUES (?, ?, ?, ?, ?, ?, ?)",
                    [
                        (
                            "snap-1",
                            "text",
                            "developer",
                            "human_authored",
                            100,
                            25,
                            b"C",
                        ),
                        (
                            "snap-1",
                            "text",
                            "developer",
                            "human_authored",
                            100,
                            25,
                            b"C",
                        ),
                    ],
                )
                connection.commit()

            report = MODULE.characterize_database(database)

        self.assertEqual(report["developer_text_estimated_tokens"]["value"], 450)
        self.assertEqual(report["cross_request_recurring_tokens"]["value"], 400)
        self.assertEqual(report["repeat_exposure_tokens"]["value"], 250)

    def test_cli_writes_the_aggregate_report_contract(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            database = root / "fixture.sqlite3"
            execution = root / "execution.json"
            output_json = root / "report.json"
            output_md = root / "report.md"
            create_fixture(database)
            execution.write_text(
                json.dumps(
                    {
                        "execution": {
                            "sessions_completed": 2,
                            "sessions_return_code_zero": 2,
                            "sessions_return_code_nonzero": 0,
                            "sessions_timed_out": 0,
                        }
                    }
                ),
                encoding="utf-8",
            )

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--metadata-database",
                    str(database),
                    "--execution-json",
                    str(execution),
                    "--sessions",
                    "2",
                    "--codex-version",
                    "codex-cli test",
                    "--output-json",
                    str(output_json),
                    "--output-md",
                    str(output_md),
                ],
                check=True,
                capture_output=True,
                text=True,
            )

            report = json.loads(output_json.read_text(encoding="utf-8"))
            rendered = output_md.read_text(encoding="utf-8")

        self.assertEqual(report["phase"], "6.2")
        self.assertEqual(
            report["shadow_gate"]["decision"],
            "source_attribution_required_before_policy_design",
        )
        self.assertIn("# TRACEPRESS_INSTRUCTION_SURFACE_SHADOW_PILOT_015", rendered)

    def test_complete_metadata_reports_cross_session_repetition_without_identities(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            database = Path(raw) / "fixture.sqlite3"
            create_fixture(database)

            report = MODULE.characterize_database(database)

        self.assertEqual(report["requests_observed"], 3)
        self.assertEqual(report["complete_observations"], 3)
        self.assertEqual(report["observation_coverage_basis_points"], 10_000)
        self.assertEqual(report["developer_text_blocks"], 5)
        self.assertEqual(report["developer_text_bytes"]["value"], 1_600)
        self.assertEqual(report["developer_text_estimated_tokens"]["value"], 400)
        self.assertEqual(report["cross_request_recurring_tokens"]["value"], 400)
        self.assertEqual(report["cross_session_recurring_tokens"]["value"], 400)
        self.assertEqual(report["repeat_exposure_tokens"]["value"], 250)
        self.assertEqual(report["repeat_exposure_share_basis_points"], 6_250)
        self.assertEqual(report["unknown_developer_blocks"], 3)
        self.assertEqual(report["unknown_developer_bytes"], 3_000)
        self.assertEqual(report["provider_usage"]["input_tokens"]["value"], 3_000)
        self.assertEqual(report["provider_usage"]["cached_input_tokens"]["value"], 2_400)
        self.assertEqual(report["provider_usage"]["uncached_input_tokens"]["value"], 600)

        encoded = json.dumps(report, sort_keys=True)
        for forbidden in (
            "exact_fingerprint",
            "semantic_path",
            "session_id",
            "request_id",
            "prompt",
            "content",
        ):
            self.assertNotIn(forbidden, encoded)

    def test_complete_repeated_surface_requires_source_attribution_not_active_policy(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            database = Path(raw) / "fixture.sqlite3"
            create_fixture(database)
            surface = MODULE.characterize_database(database)

        report = MODULE.build_report(
            {
                "execution": {
                    "sessions_completed": 2,
                    "sessions_return_code_zero": 2,
                    "sessions_return_code_nonzero": 0,
                    "sessions_timed_out": 0,
                }
            },
            surface,
            sessions_requested=2,
            codex_version_value="codex-cli test",
        )

        self.assertEqual(
            report["shadow_gate"]["decision"],
            "source_attribution_required_before_policy_design",
        )
        self.assertFalse(report["shadow_gate"]["provider_effect_active"])
        self.assertFalse(report["shadow_gate"]["instruction_policy_active"])
        self.assertEqual(report["codex_version"], "codex-cli test")


if __name__ == "__main__":
    unittest.main()
