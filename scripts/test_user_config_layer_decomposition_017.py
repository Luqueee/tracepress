#!/usr/bin/env python3
"""Contract tests for Phase 6.4 user-config layer decomposition."""

from __future__ import annotations

from contextlib import closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("run_user_config_layer_decomposition_017.py")
SPEC = importlib.util.spec_from_file_location("user_config_layer_decomposition_017", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def create_database(path: Path, extra: tuple[bytes, int] | None) -> None:
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
                correlation_status TEXT NOT NULL
            );
            CREATE TABLE context_block_occurrences(
                snapshot_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                role TEXT,
                origin TEXT NOT NULL,
                estimated_tokens INTEGER,
                exact_fingerprint BLOB
            );
            CREATE TABLE operations(operation_id TEXT PRIMARY KEY, session_id TEXT NOT NULL);
            CREATE TABLE provider_requests(request_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL);
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
            """
        )
        for index in range(2):
            suffix = f"{path.stem}-{index}"
            connection.execute(
                "INSERT INTO context_snapshots VALUES (?, ?, ?, 1, 'complete', 1, 'correlated')",
                (f"snapshot-{suffix}", f"session-{suffix}", f"request-{suffix}"),
            )
            connection.execute(
                "INSERT INTO operations VALUES (?, ?)",
                (f"operation-{suffix}", f"session-{suffix}"),
            )
            connection.execute(
                "INSERT INTO provider_requests VALUES (?, ?)",
                (f"request-{suffix}", f"operation-{suffix}"),
            )
            connection.execute(
                "INSERT INTO context_block_occurrences VALUES (?, 'text', 'developer', 'human_authored', 100, ?)",
                (f"snapshot-{suffix}", b"COMMON"),
            )
            if extra is not None:
                connection.execute(
                    "INSERT INTO context_block_occurrences VALUES (?, 'text', 'developer', 'human_authored', ?, ?)",
                    (f"snapshot-{suffix}", extra[1], extra[0]),
                )
            connection.execute(
                "INSERT INTO provider_attempts VALUES (?, ?, 0)",
                (f"attempt-{suffix}", f"request-{suffix}"),
            )
            connection.execute(
                "INSERT INTO provider_usage VALUES (?, 1000, 800, 200, 50, 20)",
                (f"attempt-{suffix}",),
            )
        connection.commit()


def execution_for(name: str, round_index: int) -> dict[str, object]:
    schedule_position = MODULE.interleaved_schedule(round_index + 1)[round_index].index(name)
    return {
        "model": MODULE.MODEL,
        "repository_pin": {
            "repository": MODULE.RIPGREP_REPOSITORY,
            "commit_sha": MODULE.RIPGREP_SHA,
        },
        "forwarding_mutations": 0,
        "active_compression": "off",
        "shadow_compression": False,
        "codex_user_config_loaded": name != "ignore_user_config",
        "workspace_instruction_profile": "none",
        "developer_instruction_profile": "none",
        "codex_disabled_features": sorted(MODULE.ARM_FEATURES[name]),
        "codex_ephemeral": True,
        "codex_sandbox_profile": "read_only",
        "codex_approval_policy": "never",
        "search_pattern_start": round_index,
        "experiment_round": round_index,
        "schedule_position": schedule_position,
        "execution": {
            "sessions_completed": 1,
            "sessions_return_code_zero": 1,
            "sessions_return_code_nonzero": 0,
            "sessions_timed_out": 0,
        },
    }


class UserConfigLayerDecompositionTests(unittest.TestCase):
    def test_persisted_report_is_aggregate_only_and_keeps_provider_policy_blocked(self) -> None:
        path = (
            SCRIPT.parents[1]
            / "reports/user-config-layer-decomposition-017/TRACEPRESS_USER_CONFIG_LAYER_DECOMPOSITION_017.json"
        )
        report = json.loads(path.read_text(encoding="utf-8"))

        attribution = report["layer_decomposition"]["attribution"]
        self.assertEqual(attribution["plugins_removed_tokens_per_request"], 9_973)
        self.assertEqual(attribution["plugins_added_tokens_per_request"], 1_540)
        self.assertEqual(attribution["memories_removed_tokens_per_request"], 5_711)
        self.assertEqual(attribution["hooks_removed_tokens_per_request"], 0)
        self.assertEqual(attribution["combined_removed_tokens_per_request"], 15_684)
        self.assertTrue(report["gate"]["provider_cache_aa_within_threshold"])
        self.assertFalse(
            report["gate"]["provider_cache_null_control_within_threshold"]
        )
        self.assertFalse(report["gate"]["provider_comparison_reliable"])
        self.assertFalse(report["gate"]["instruction_policy_active"])
        serialized = json.dumps(report)
        self.assertNotIn("/home/", serialized)
        self.assertNotIn("fingerprint", serialized.lower())
        self.assertNotIn("session_id", serialized)
        self.assertNotIn("request_id", serialized)

    def test_known_layer_removal_is_attributed_without_identity_leakage(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            databases: dict[str, list[Path]] = {}
            executions: dict[str, list[dict[str, object]]] = {}
            for name in MODULE.ARM_NAMES:
                extra = None if name in {"no_plugins", "no_extensions", "ignore_user_config"} else (b"PLUGIN", 40)
                databases[name] = []
                executions[name] = []
                for round_index in range(2):
                    database = root / f"{name}-{round_index}.sqlite3"
                    create_database(database, extra)
                    databases[name].append(database)
                    executions[name].append(execution_for(name, round_index))

            decomposition = MODULE.decompose_arms(databases)
            report = MODULE.build_report(
                executions,
                decomposition,
                rounds=2,
                codex_version_value="codex-cli test",
            )

            executions["no_plugins"][0]["schedule_position"] = 9
            rejected = MODULE.build_report(
                executions,
                decomposition,
                rounds=2,
                codex_version_value="codex-cli test",
            )

        layers = report["layer_decomposition"]["attribution"]
        self.assertEqual(layers["plugins_removed_tokens_per_request"], 40)
        self.assertEqual(layers["memories_removed_tokens_per_request"], 0)
        self.assertEqual(layers["hooks_removed_tokens_per_request"], 0)
        self.assertEqual(layers["combined_removed_tokens_per_request"], 40)
        self.assertTrue(report["gate"]["baseline_aa_structural_match"])
        self.assertTrue(report["gate"]["provider_cache_null_control_within_threshold"])
        self.assertTrue(report["gate"]["provider_comparison_reliable"])
        self.assertEqual(report["gate"]["decision"], "structural_decomposition_complete_policy_blocked")
        serialized = json.dumps(report)
        self.assertNotIn("PLUGIN", serialized)
        self.assertNotIn("fingerprint", serialized.lower())
        self.assertNotIn("session-", serialized)
        self.assertNotIn("request-", serialized)
        self.assertEqual(
            rejected["gate"]["decision"], "insufficient_decomposition_evidence"
        )

    def test_interleaved_schedule_keeps_aa_adjacent_and_varies_order(self) -> None:
        schedule = MODULE.interleaved_schedule(3)
        self.assertEqual(len(schedule), 3)
        self.assertTrue(all(set(round_) == set(MODULE.ARM_NAMES) for round_ in schedule))
        for round_ in schedule:
            self.assertEqual(abs(round_.index("user_config_a") - round_.index("user_config_b")), 1)
        self.assertNotEqual(schedule[0], schedule[1])

    def test_structural_aa_drift_is_reported_and_blocks_attribution(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            databases: dict[str, list[Path]] = {}
            for name in MODULE.ARM_NAMES:
                database = root / f"{name}.sqlite3"
                extra = (b"BASE_A", 40) if name == "user_config_a" else (b"BASE_B", 30)
                create_database(database, extra)
                databases[name] = [database]

            result = MODULE.decompose_arms(databases)

        self.assertFalse(result["integrity"]["baseline_aa_structural_match"])
        self.assertEqual(result["baseline_aa"]["a_only_tokens_per_request"], 40)
        self.assertEqual(result["baseline_aa"]["b_only_tokens_per_request"], 30)
        self.assertIsNone(
            result["attribution"]["plugins_removed_tokens_per_request"]
        )


if __name__ == "__main__":
    unittest.main()
