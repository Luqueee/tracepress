#!/usr/bin/env python3
"""Contract tests for Phase 6.3 instruction source attribution."""

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


SCRIPT = Path(__file__).with_name("run_instruction_source_attribution_016.py")
SPEC = importlib.util.spec_from_file_location("instruction_source_attribution_016", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def create_arm(path: Path, extra: tuple[bytes, int] | None) -> None:
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
        for index in range(1, 3):
            session = f"session-{index}"
            request = f"request-{index}"
            snapshot = f"snapshot-{index}"
            operation = f"operation-{index}"
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
                fingerprint, tokens = extra
                connection.execute(
                    "INSERT INTO context_block_occurrences VALUES (?, 'text', 'developer', 'human_authored', ?, ?)",
                    (snapshot, tokens, fingerprint),
                )
            connection.execute(
                "INSERT INTO provider_attempts VALUES (?, ?, 0)",
                (f"attempt-{index}", request),
            )
            connection.execute(
                "INSERT INTO provider_usage VALUES (?, 1000, 800, 200, 50, 20)",
                (f"attempt-{index}",),
            )
        connection.commit()


def object_keys(value: object) -> set[str]:
    if isinstance(value, dict):
        return set(value).union(*(object_keys(item) for item in value.values()))
    if isinstance(value, list):
        return set().union(*(object_keys(item) for item in value))
    return set()


def execution_for(name: str) -> dict[str, object]:
    return {
        "model": "gpt-5.6-luna",
        "repository_pin": {
            "repository": "BurntSushi/ripgrep",
            "commit_sha": "3fce3b5bb0236da2df6d99672afb8a719642eca7",
        },
        "forwarding_mutations": 0,
        "active_compression": "off",
        "shadow_compression": False,
        "codex_user_config_loaded": name == "user_config",
        "workspace_instruction_profile": (
            "public_marker_v1"
            if name == "ignore_user_config_workspace_marker"
            else "none"
        ),
        "developer_instruction_profile": (
            "public_marker_v1"
            if name == "ignore_user_config_developer_marker"
            else "none"
        ),
        "execution": {
            "sessions_completed": 2,
            "sessions_return_code_zero": 2,
            "sessions_return_code_nonzero": 0,
            "sessions_timed_out": 0,
        },
    }


class InstructionSourceAttributionContractTests(unittest.TestCase):
    def test_persisted_report_matches_aggregate_privacy_contract(self) -> None:
        report_path = (
            SCRIPT.parents[1]
            / "reports/instruction-source-attribution-016/TRACEPRESS_INSTRUCTION_SOURCE_ATTRIBUTION_016.json"
        )
        report = json.loads(report_path.read_text(encoding="utf-8"))

        self.assertEqual(report["sessions_per_arm"], 3)
        self.assertEqual(
            report["source_attribution"]["attribution"][
                "user_config_only_tokens_per_request"
            ],
            15_749,
        )
        self.assertEqual(
            report["source_attribution"]["attribution"][
                "developer_marker_only_tokens_per_request"
            ],
            57,
        )
        self.assertEqual(
            report["gate"]["decision"], "attribution_calibrated_policy_still_blocked"
        )
        keys = object_keys(report)
        for forbidden in (
            "exact_fingerprint",
            "semantic_path",
            "session_id",
            "request_id",
            "instruction_text",
            "authentication_token",
        ):
            self.assertNotIn(forbidden, keys)

    def test_cli_writes_an_aggregate_attribution_report(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            create_arm(root / "user_config.sqlite3", (b"USER_CONFIG", 40))
            create_arm(root / "ignore_user_config.sqlite3", None)
            create_arm(
                root / "ignore_user_config_workspace_marker.sqlite3",
                (b"WORKSPACE", 25),
            )
            create_arm(
                root / "ignore_user_config_developer_marker.sqlite3",
                (b"DEVELOPER", 30),
            )
            for name in MODULE.ARM_NAMES:
                (root / f"{name}.json").write_text(
                    json.dumps(execution_for(name)),
                    encoding="utf-8",
                )
            output_json = root / "report.json"
            output_md = root / "report.md"

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--arm-root",
                    str(root),
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

        self.assertEqual(report["phase"], "6.3")
        self.assertEqual(
            report["gate"]["decision"], "attribution_calibrated_policy_still_blocked"
        )
        self.assertIn("# TRACEPRESS_INSTRUCTION_SOURCE_ATTRIBUTION_016", rendered)

    def test_calibrated_attribution_still_blocks_active_policy(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            user = root / "user.sqlite3"
            ignored = root / "ignored.sqlite3"
            workspace = root / "workspace.sqlite3"
            developer = root / "developer.sqlite3"
            create_arm(user, (b"USER_CONFIG", 40))
            create_arm(ignored, None)
            create_arm(workspace, (b"WORKSPACE", 25))
            create_arm(developer, (b"DEVELOPER", 30))
            attribution = MODULE.attribute_databases(
                {
                    "user_config": user,
                    "ignore_user_config": ignored,
                    "ignore_user_config_workspace_marker": workspace,
                    "ignore_user_config_developer_marker": developer,
                }
            )

        executions = {name: execution_for(name) for name in MODULE.ARM_NAMES}
        report = MODULE.build_report(
            executions,
            attribution,
            sessions_requested=2,
            codex_version_value="codex-cli test",
        )

        self.assertEqual(
            report["gate"]["decision"], "attribution_calibrated_policy_still_blocked"
        )
        self.assertFalse(report["gate"]["instruction_policy_active"])
        self.assertFalse(report["gate"]["provider_effect_active"])
        self.assertEqual(report["codex_version"], "codex-cli test")

    def test_misconfigured_arm_cannot_pass_attribution_gate(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            create_arm(root / "user.sqlite3", (b"USER_CONFIG", 40))
            create_arm(root / "ignored.sqlite3", None)
            create_arm(root / "workspace.sqlite3", (b"WORKSPACE", 25))
            create_arm(root / "developer.sqlite3", (b"DEVELOPER", 30))
            attribution = MODULE.attribute_databases(
                {
                    "user_config": root / "user.sqlite3",
                    "ignore_user_config": root / "ignored.sqlite3",
                    "ignore_user_config_workspace_marker": root / "workspace.sqlite3",
                    "ignore_user_config_developer_marker": root / "developer.sqlite3",
                }
            )
        executions = {name: execution_for(name) for name in MODULE.ARM_NAMES}
        executions["user_config"]["codex_user_config_loaded"] = False

        report = MODULE.build_report(
            executions,
            attribution,
            sessions_requested=2,
            codex_version_value="codex-cli test",
        )

        self.assertEqual(report["gate"]["decision"], "insufficient_attribution_evidence")

    def test_known_workspace_intervention_is_attributed_without_exposing_identity(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            user = root / "user.sqlite3"
            ignored = root / "ignored.sqlite3"
            workspace = root / "workspace.sqlite3"
            developer = root / "developer.sqlite3"
            create_arm(user, (b"USER_CONFIG", 40))
            create_arm(ignored, None)
            create_arm(workspace, (b"WORKSPACE", 25))
            create_arm(developer, (b"DEVELOPER", 30))

            result = MODULE.attribute_databases(
                {
                    "user_config": user,
                    "ignore_user_config": ignored,
                    "ignore_user_config_workspace_marker": workspace,
                    "ignore_user_config_developer_marker": developer,
                }
            )

        self.assertEqual(result["arms"]["user_config"]["stable_tokens_per_request"], 140)
        self.assertEqual(result["arms"]["ignore_user_config"]["stable_tokens_per_request"], 100)
        self.assertEqual(
            result["arms"]["ignore_user_config_workspace_marker"]["stable_tokens_per_request"],
            125,
        )
        self.assertEqual(
            result["arms"]["ignore_user_config_developer_marker"]["stable_tokens_per_request"],
            130,
        )
        self.assertEqual(result["attribution"]["common_all_arms_tokens_per_request"], 100)
        self.assertEqual(result["attribution"]["user_config_only_tokens_per_request"], 40)
        self.assertEqual(result["attribution"]["workspace_marker_only_tokens_per_request"], 25)
        self.assertEqual(result["attribution"]["workspace_marker_removed_tokens_per_request"], 0)
        self.assertEqual(result["attribution"]["developer_marker_only_tokens_per_request"], 30)
        self.assertEqual(result["attribution"]["developer_marker_removed_tokens_per_request"], 0)
        self.assertTrue(result["calibration"]["developer_marker_delta_observed"])

        encoded = json.dumps(result, sort_keys=True)
        for forbidden in (
            "exact_fingerprint",
            "semantic_path",
            "session_id",
            "request_id",
            "USER_CONFIG",
            "WORKSPACE",
            "DEVELOPER",
        ):
            self.assertNotIn(forbidden, encoded)


if __name__ == "__main__":
    unittest.main()
