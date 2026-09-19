#!/usr/bin/env python3
"""Contract tests for the Phase 6.6 cache-key attribution study."""

from __future__ import annotations

from contextlib import closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("run_cache_key_attribution_019.py")
SPEC = importlib.util.spec_from_file_location("cache_key_attribution_019", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def create_database(path: Path, uncached_per_request: int, *, changed_prefix: bool = False) -> None:
    with closing(sqlite3.connect(path)) as connection:
        connection.executescript(
            """
            CREATE TABLE operations(
                operation_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, started_at TEXT NOT NULL
            );
            CREATE TABLE provider_requests(
                request_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL, request_bytes INTEGER,
                model TEXT, stream INTEGER, background INTEGER, store INTEGER,
                reasoning_effort TEXT, text_verbosity TEXT, truncation TEXT,
                previous_response_id_present INTEGER, input_item_count INTEGER,
                tool_count INTEGER, text_input_block_count INTEGER,
                image_input_block_count INTEGER, file_input_block_count INTEGER,
                observation_status TEXT
            );
            CREATE TABLE provider_attempts(
                attempt_id TEXT PRIMARY KEY, request_id TEXT NOT NULL, ordinal INTEGER NOT NULL
            );
            CREATE TABLE provider_usage(
                attempt_id TEXT PRIMARY KEY, input_total INTEGER, input_cached INTEGER,
                input_uncached INTEGER, output_total INTEGER, output_reasoning INTEGER
            );
            CREATE TABLE context_snapshots(
                snapshot_id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                provider_request_id TEXT NOT NULL, analysis_version INTEGER NOT NULL,
                status TEXT NOT NULL, explicit_request_complete INTEGER NOT NULL,
                correlation_status TEXT NOT NULL, uses_previous_response INTEGER,
                uses_conversation_state INTEGER, uses_item_references INTEGER,
                uses_prompt_reference INTEGER, uses_external_files INTEGER,
                uses_external_images INTEGER, contains_opaque_items INTEGER,
                logical_context_status TEXT, duplicate_key_detected INTEGER
            );
            CREATE TABLE context_block_occurrences(
                snapshot_id TEXT NOT NULL, ordinal INTEGER NOT NULL, kind TEXT NOT NULL,
                role TEXT, origin TEXT NOT NULL, semantic_path TEXT, raw_bytes INTEGER,
                exact_fingerprint BLOB, estimated_tokens INTEGER, detected_kind TEXT,
                tool_name TEXT
            );
            """
        )
        for index in range(2):
            suffix = f"{path.stem}-{index}"
            operation = f"operation-{suffix}"
            request = f"request-{suffix}"
            attempt = f"attempt-{suffix}"
            snapshot = f"snapshot-{suffix}"
            connection.execute(
                "INSERT INTO operations VALUES (?, ?, ?)",
                (operation, f"session-{suffix}", f"2026-01-01T00:00:0{index}Z"),
            )
            connection.execute(
                "INSERT INTO provider_requests VALUES (?, ?, ?, ?, 1, 0, 0, ?, ?, ?, 0, 2, 1, 2, 0, 0, 'complete')",
                (request, operation, 1000, MODULE.MODEL, "medium", "medium", "auto"),
            )
            connection.execute(
                "INSERT INTO provider_attempts VALUES (?, ?, 0)", (attempt, request)
            )
            connection.execute(
                "INSERT INTO provider_usage VALUES (?, 1000, ?, ?, 50, 20)",
                (attempt, 1000 - uncached_per_request, uncached_per_request),
            )
            connection.execute(
                "INSERT INTO context_snapshots VALUES (?, ?, ?, 1, 'complete', 1, 'correlated', 0, 0, 0, 0, 0, 0, 0, 'explicit_complete', 0)",
                (snapshot, f"session-{suffix}", request),
            )
            fingerprint = b"CHANGED" if changed_prefix and index == 0 else b"COMMON"
            connection.execute(
                "INSERT INTO context_block_occurrences VALUES (?, 0, 'text', 'developer', 'human_authored', 'instructions', 400, ?, 100, 'plain_text', NULL)",
                (snapshot, fingerprint),
            )
        connection.commit()


def execution(name: str, round_index: int) -> dict[str, object]:
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
        "codex_enabled_features": sorted(MODULE.ARM_ENABLED[name]),
        "codex_disabled_features": sorted(MODULE.ARM_DISABLED[name]),
        "codex_ephemeral": True,
        "codex_sandbox_profile": "read_only",
        "codex_approval_policy": "never",
        "search_pattern_start": round_index // 3,
        "experiment_round": round_index,
        "schedule_position": MODULE.balanced_schedule()[round_index].index(name),
        "execution": {
            "sessions_completed": 1,
            "sessions_return_code_zero": 1,
            "sessions_return_code_nonzero": 0,
            "sessions_timed_out": 0,
        },
    }


class CacheKeyAttributionTests(unittest.TestCase):
    def test_balanced_schedule_covers_positions_and_transitions(self) -> None:
        schedule = MODULE.balanced_schedule()
        for name in MODULE.ARM_NAMES:
            self.assertEqual(
                [sum(order[position] == name for order in schedule) for position in range(3)],
                [2, 2, 2],
            )
        transitions = [(order[i], order[i + 1]) for order in schedule for i in range(2)]
        for left in MODULE.ARM_NAMES:
            for right in MODULE.ARM_NAMES:
                if left != right:
                    self.assertEqual(transitions.count((left, right)), 2)

    def test_effective_hooks_state_is_attributed_only_after_prefix_gate(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            databases = {name: [] for name in MODULE.ARM_NAMES}
            for name in MODULE.ARM_NAMES:
                uncached = 100 if name != "hooks_explicit_disabled" else 300
                for index in range(6):
                    path = root / f"{name}-{index}.sqlite3"
                    create_database(path, uncached)
                    databases[name].append(path)
            result = MODULE.analyze(databases)

        self.assertTrue(result["integrity"]["observable_prefix_equivalent"])
        self.assertEqual(result["observable_prefix_differences"]["divergent_block_classes"], [])
        self.assertTrue(result["cache_attribution"]["enabled_pair_equivalent"])
        self.assertTrue(result["cache_attribution"]["disabled_separated"])
        self.assertEqual(
            result["cache_attribution"]["classification"],
            "effective_hooks_state_associated",
        )

    def test_prefix_difference_blocks_cache_attribution(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            databases = {name: [] for name in MODULE.ARM_NAMES}
            for name in MODULE.ARM_NAMES:
                for index in range(6):
                    path = root / f"{name}-{index}.sqlite3"
                    create_database(
                        path,
                        300 if name == "hooks_explicit_disabled" else 100,
                        changed_prefix=name == "hooks_explicit_disabled",
                    )
                    databases[name].append(path)
            result = MODULE.analyze(databases)

        self.assertFalse(result["integrity"]["observable_prefix_equivalent"])
        differences = result["observable_prefix_differences"]
        self.assertEqual(
            differences["block_field_mismatch_cells"]["exact_fingerprint"], 6
        )
        self.assertEqual(
            differences["divergent_block_classes"][0]["semantic_path"], "instructions"
        )
        self.assertEqual(
            result["cache_attribution"]["classification"],
            "blocked_by_observable_prefix_difference",
        )

    def test_execution_contract_and_report_privacy(self) -> None:
        executions = {
            name: [execution(name, index) for index in range(6)]
            for name in MODULE.ARM_NAMES
        }
        analysis = {
            "integrity": {
                "all_measurements_complete": True,
                "observable_prefix_equivalent": True,
            },
            "cache_attribution": {"classification": "effective_hooks_state_associated"},
        }
        report = MODULE.build_report(executions, analysis, "codex-cli test")
        serialized = json.dumps(report)

        self.assertTrue(report["gate"]["evidence_complete"])
        self.assertFalse(report["gate"]["cache_key_value_observed"])
        self.assertEqual(report["gate"]["decision"], "effective_hooks_state_associated")
        for canary in ("PROMPT_CANARY", "CACHE_KEY_CANARY", "AUTH_CANARY"):
            self.assertNotIn(canary, serialized)

    def test_persisted_report_keeps_cache_key_values_out(self) -> None:
        path = (
            SCRIPT.parents[1]
            / "reports/cache-key-attribution-019/TRACEPRESS_CACHE_KEY_ATTRIBUTION_019.json"
        )
        report = json.loads(path.read_text(encoding="utf-8"))
        self.assertFalse(report["gate"]["observable_prefix_equivalent"])
        self.assertFalse(report["gate"]["cache_key_value_observed"])
        self.assertEqual(
            report["gate"]["decision"], "blocked_by_observable_prefix_difference"
        )
        self.assertAlmostEqual(
            report["attribution"]["cache_attribution"][
                "explicit_enabled_vs_disabled_uncached_delta"
            ],
            0.1115,
            places=4,
        )

    def test_workload_supports_explicit_enable_without_conflicting_override(self) -> None:
        workload_path = SCRIPT.with_name("characterize_public_provider_native_search_001.py")
        spec = importlib.util.spec_from_file_location("public_search_workload", workload_path)
        assert spec is not None and spec.loader is not None
        workload = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(workload)
        command = workload.codex_exec_command(
            Path("tracepress"), "safe prompt", ignore_user_config=False,
            enabled_features=("hooks",),
        )
        self.assertIn("--enable", command)
        self.assertEqual(command[command.index("--enable") + 1], "hooks")
        with self.assertRaises(ValueError):
            workload.codex_exec_command(
                Path("tracepress"), "safe prompt", ignore_user_config=False,
                enabled_features=("hooks",), disabled_features=("hooks",),
            )


if __name__ == "__main__":
    unittest.main()
