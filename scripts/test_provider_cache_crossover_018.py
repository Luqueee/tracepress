#!/usr/bin/env python3
"""Contract tests for the Phase 6.5 provider-cache crossover."""

from __future__ import annotations

from contextlib import closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("run_provider_cache_crossover_018.py")
SPEC = importlib.util.spec_from_file_location("provider_cache_crossover_018", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def create_database(
    path: Path, extra: tuple[bytes, int] | None, uncached_per_request: int
) -> None:
    with closing(sqlite3.connect(path)) as connection:
        connection.executescript(
            """
            CREATE TABLE context_snapshots(
                snapshot_id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                provider_request_id TEXT NOT NULL, analysis_version INTEGER NOT NULL,
                status TEXT NOT NULL, explicit_request_complete INTEGER NOT NULL,
                correlation_status TEXT NOT NULL
            );
            CREATE TABLE context_block_occurrences(
                snapshot_id TEXT NOT NULL, kind TEXT NOT NULL, role TEXT,
                origin TEXT NOT NULL, estimated_tokens INTEGER, exact_fingerprint BLOB
            );
            CREATE TABLE operations(operation_id TEXT PRIMARY KEY, session_id TEXT NOT NULL);
            CREATE TABLE provider_requests(request_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL);
            CREATE TABLE provider_attempts(
                attempt_id TEXT PRIMARY KEY, request_id TEXT NOT NULL, ordinal INTEGER NOT NULL
            );
            CREATE TABLE provider_usage(
                attempt_id TEXT PRIMARY KEY, input_total INTEGER, input_cached INTEGER,
                input_uncached INTEGER, output_total INTEGER, output_reasoning INTEGER
            );
            """
        )
        for index in range(2):
            suffix = f"{path.stem}-{index}"
            snapshot = f"snapshot-{suffix}"
            session = f"session-{suffix}"
            request = f"request-{suffix}"
            operation = f"operation-{suffix}"
            attempt = f"attempt-{suffix}"
            connection.execute(
                "INSERT INTO context_snapshots VALUES (?, ?, ?, 1, 'complete', 1, 'correlated')",
                (snapshot, session, request),
            )
            connection.execute("INSERT INTO operations VALUES (?, ?)", (operation, session))
            connection.execute("INSERT INTO provider_requests VALUES (?, ?)", (request, operation))
            connection.execute(
                "INSERT INTO context_block_occurrences VALUES (?, 'text', 'developer', 'human_authored', 100, ?)",
                (snapshot, b"COMMON"),
            )
            if extra is not None:
                connection.execute(
                    "INSERT INTO context_block_occurrences VALUES (?, 'text', 'developer', 'human_authored', ?, ?)",
                    (snapshot, extra[1], extra[0]),
                )
            connection.execute(
                "INSERT INTO provider_attempts VALUES (?, ?, 0)", (attempt, request)
            )
            connection.execute(
                "INSERT INTO provider_usage VALUES (?, 1000, ?, ?, 50, 20)",
                (attempt, 1000 - uncached_per_request, uncached_per_request),
            )
        connection.commit()


def execution_for(name: str, round_index: int) -> dict[str, object]:
    position = MODULE.balanced_crossover_schedule()[round_index].index(name)
    return {
        "model": MODULE.MODEL,
        "repository_pin": {
            "repository": MODULE.RIPGREP_REPOSITORY,
            "commit_sha": MODULE.RIPGREP_SHA,
        },
        "forwarding_mutations": 0,
        "active_compression": "off",
        "shadow_compression": False,
        "codex_user_config_loaded": True,
        "workspace_instruction_profile": "none",
        "developer_instruction_profile": "none",
        "codex_disabled_features": sorted(MODULE.ARM_FEATURES[name]),
        "codex_ephemeral": True,
        "codex_sandbox_profile": "read_only",
        "codex_approval_policy": "never",
        "search_pattern_start": round_index // 3,
        "experiment_round": round_index,
        "schedule_position": position,
        "execution": {
            "sessions_completed": 1,
            "sessions_return_code_zero": 1,
            "sessions_return_code_nonzero": 0,
            "sessions_timed_out": 0,
        },
    }


class ProviderCacheCrossoverTests(unittest.TestCase):
    def test_persisted_report_separates_total_input_from_cache_reliability(self) -> None:
        path = (
            SCRIPT.parents[1]
            / "reports/provider-cache-crossover-018/TRACEPRESS_PROVIDER_CACHE_CROSSOVER_018.json"
        )
        report = json.loads(path.read_text(encoding="utf-8"))
        controls = report["crossover"]["cache_controls"]

        self.assertTrue(controls["total_input_comparison_reliable"])
        self.assertFalse(controls["cache_split_comparison_reliable"])
        self.assertFalse(controls["provider_comparison_reliable"])
        self.assertLess(controls["null_input_total_relative_delta"], 0.001)
        self.assertGreater(controls["null_uncached_aggregate_relative_delta"], 1.2)
        self.assertEqual(
            report["gate"]["decision"],
            "crossover_baseline_failed_provider_reliability",
        )
        self.assertFalse(report["gate"]["instruction_policy_active"])
        serialized = json.dumps(report)
        self.assertNotIn("/home/", serialized)
        self.assertNotIn("fingerprint", serialized.lower())
        self.assertNotIn("session_id", serialized)
        self.assertNotIn("request_id", serialized)

    def test_schedule_balances_positions_and_directed_transitions(self) -> None:
        schedule = MODULE.balanced_crossover_schedule()

        self.assertEqual(len(schedule), 6)
        for name in MODULE.ARM_NAMES:
            self.assertEqual(
                [row.index(name) for row in schedule].count(0), 2
            )
            self.assertEqual(
                [row.index(name) for row in schedule].count(1), 2
            )
            self.assertEqual(
                [row.index(name) for row in schedule].count(2), 2
            )
        transitions = [
            (row[index], row[index + 1])
            for row in schedule
            for index in range(len(row) - 1)
        ]
        for left in MODULE.ARM_NAMES:
            for right in MODULE.ARM_NAMES:
                if left != right:
                    self.assertEqual(transitions.count((left, right)), 2)
        for task_index in range(2):
            for name in MODULE.ARM_NAMES:
                for position in range(3):
                    matches = [
                        round_index
                        for round_index, row in enumerate(schedule)
                        if round_index // 3 == task_index and row[position] == name
                    ]
                    self.assertEqual(len(matches), 1)

    def test_structural_null_and_provider_gate_use_balanced_cells(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            databases: dict[str, list[Path]] = {name: [] for name in MODULE.ARM_NAMES}
            executions: dict[str, list[dict[str, object]]] = {
                name: [] for name in MODULE.ARM_NAMES
            }
            for round_index in range(6):
                for name in MODULE.ARM_NAMES:
                    database = root / f"{name}-{round_index}.sqlite3"
                    extra = None if name == "no_extensions_treatment" else (b"BASE", 40)
                    uncached = 100 if name == "no_extensions_treatment" else 200
                    create_database(database, extra, uncached)
                    databases[name].append(database)
                    executions[name].append(execution_for(name, round_index))

            analysis = MODULE.analyze_crossover(databases)
            report = MODULE.build_report(
                executions, analysis, codex_version_value="codex-cli test"
            )
            executions["no_hooks_null"][0]["schedule_position"] = 9
            rejected = MODULE.build_report(
                executions, analysis, codex_version_value="codex-cli test"
            )

        self.assertTrue(analysis["integrity"]["control_null_structural_match"])
        self.assertTrue(analysis["cache_controls"]["provider_comparison_reliable"])
        self.assertTrue(
            analysis["cache_controls"]["total_input_comparison_reliable"]
        )
        self.assertTrue(
            analysis["cache_controls"]["cache_split_comparison_reliable"]
        )
        self.assertEqual(
            analysis["treatment_effect"]["input_total_relative_change"], 0.0
        )
        self.assertEqual(
            analysis["treatment_effect"]["uncached_input_relative_change"], -0.5
        )
        self.assertEqual(
            report["gate"]["decision"], "crossover_baseline_complete_policy_blocked"
        )
        self.assertFalse(report["gate"]["instruction_policy_active"])
        self.assertEqual(
            rejected["gate"]["decision"], "insufficient_crossover_evidence"
        )
        serialized = json.dumps(report)
        self.assertNotIn("BASE", serialized)
        self.assertNotIn("fingerprint", serialized.lower())
        self.assertNotIn("session-", serialized)
        self.assertNotIn("request-", serialized)


if __name__ == "__main__":
    unittest.main()
