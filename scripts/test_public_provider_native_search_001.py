#!/usr/bin/env python3
"""Compatibility tests for controlled public Search workload options."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("characterize_public_provider_native_search_001.py")
SPEC = importlib.util.spec_from_file_location("public_search_001", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PublicSearchWorkloadTests(unittest.TestCase):
    def test_ignore_user_config_is_an_explicit_safe_exec_option(self) -> None:
        command = MODULE.codex_exec_command(
            Path("/tmp/tracepress"), "public prompt", ignore_user_config=True
        )

        self.assertEqual(
            command,
            [
                "/tmp/tracepress",
                "run",
                "codex",
                "exec",
                "--ignore-user-config",
                "-m",
                "gpt-5.6-luna",
                "-s",
                "read-only",
                "--skip-git-repo-check",
                "public prompt",
            ],
        )
        self.assertNotIn("--dangerously-bypass-approvals-and-sandbox", command)

    def test_workspace_marker_is_fixed_public_content(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            MODULE.install_workspace_instruction_marker(root)
            marker = (root / "AGENTS.md").read_text(encoding="utf-8")

        self.assertEqual(marker, MODULE.WORKSPACE_INSTRUCTION_MARKER_V1)
        self.assertIn("Tracepress public attribution marker v1", marker)

    def test_controlled_developer_marker_uses_strict_config_without_bypass(self) -> None:
        command = MODULE.codex_exec_command(
            Path("/tmp/tracepress"),
            "public prompt",
            ignore_user_config=True,
            controlled_developer_instructions=True,
        )

        self.assertIn("--ignore-user-config", command)
        self.assertIn("--strict-config", command)
        config_index = command.index("-c")
        self.assertTrue(command[config_index + 1].startswith("developer_instructions="))
        self.assertIn("Tracepress public developer marker v1", command[config_index + 1])
        self.assertNotIn("--dangerously-bypass-approvals-and-sandbox", command)


if __name__ == "__main__":
    unittest.main()
