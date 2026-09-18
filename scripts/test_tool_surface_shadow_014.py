#!/usr/bin/env python3
"""Contract tests for the Phase 6.1 Tool Surface Shadow report."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("run_tool_surface_shadow_014.py")
SPEC = importlib.util.spec_from_file_location("tool_surface_shadow_014", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def metric(value: int | float | None, source: str) -> dict[str, object]:
    return {"value": value, "source": source}


def summary() -> dict[str, object]:
    return {
        "mode": "shadow",
        "provider_effect_active": False,
        "requests_observed": 20,
        "schema_observations": 20,
        "schema_observation_coverage_basis_points": 10_000,
        "tool_definitions_exposed": metric(80, "locally_estimated"),
        "schema_bytes": metric(40_000, "locally_estimated"),
        "estimated_schema_tokens": metric(10_000, "locally_estimated"),
        "repeated_schema_tokens": metric(7_500, "locally_estimated"),
        "repeated_schema_share_basis_points": 7_500,
        "tool_calls_observed": 10,
        "distinct_defined_tools": 40,
        "distinct_used_tools": 10,
        "unused_tools_lower_bound": 30,
        "provider_usage": {
            "input_tokens": metric(50_000, "provider_reported"),
            "cached_input_tokens": metric(30_000, "provider_reported"),
            "uncached_input_tokens": metric(20_000, "provider_reported"),
            "cache_ratio": metric(0.6, "provider_reported"),
            "output_tokens": metric(2_000, "provider_reported"),
            "reasoning_tokens": metric(1_000, "provider_reported"),
        },
    }


def object_keys(value: object) -> set[str]:
    if isinstance(value, dict):
        return set(value).union(*(object_keys(item) for item in value.values()))
    if isinstance(value, list):
        return set().union(*(object_keys(item) for item in value))
    return set()


class ToolSurfaceShadowContractTests(unittest.TestCase):
    def test_persisted_report_matches_contract_and_privacy_boundary(self) -> None:
        report_path = (
            SCRIPT.parents[1]
            / "reports/tool-surface-shadow-014/TRACEPRESS_TOOL_SURFACE_SHADOW_PILOT_014.json"
        )
        report = json.loads(report_path.read_text(encoding="utf-8"))

        MODULE.validate_summary(report["tool_surface"])
        self.assertEqual(
            report["shadow_gate"]["decision"],
            "reject_active_tool_selection_no_observed_schema_surface",
        )
        self.assertEqual(report["codex_version"], "codex-cli 0.155.0")
        keys = object_keys(report)
        for forbidden in (
            "tool_name",
            "tool_name_hash",
            "schema_body",
            "arguments",
            "session_id",
            "request_id",
            "command_output",
        ):
            self.assertNotIn(forbidden, keys)

    def test_complete_shadow_evidence_passes_readiness_gate(self) -> None:
        public = {
            "execution": {
                "sessions_completed": 10,
                "sessions_return_code_zero": 10,
                "sessions_return_code_nonzero": 0,
                "sessions_timed_out": 0,
            }
        }
        value = MODULE.build_report(
            public, MODULE.validate_summary(summary()), 10, "codex-cli test"
        )
        self.assertEqual(
            value["shadow_gate"]["decision"], "shadow_evidence_ready_for_policy_design"
        )
        self.assertFalse(value["shadow_gate"]["provider_effect_active"])
        self.assertFalse(value["shadow_gate"]["tool_selection_active"])

    def test_complete_known_zero_exposure_is_a_negative_result(self) -> None:
        surface = summary()
        for field in (
            "tool_definitions_exposed",
            "schema_bytes",
            "estimated_schema_tokens",
            "repeated_schema_tokens",
        ):
            surface[field] = metric(0, "locally_estimated")
        surface["repeated_schema_share_basis_points"] = None
        public = {
            "execution": {
                "sessions_completed": 10,
                "sessions_return_code_zero": 10,
                "sessions_return_code_nonzero": 0,
                "sessions_timed_out": 0,
            }
        }

        value = MODULE.build_report(
            public, MODULE.validate_summary(surface), 10, "codex-cli test"
        )

        self.assertTrue(value["shadow_gate"]["schema_coverage_complete"])
        self.assertFalse(value["shadow_gate"]["material_schema_exposure_observed"])
        self.assertEqual(
            value["shadow_gate"]["decision"],
            "reject_active_tool_selection_no_observed_schema_surface",
        )

    def test_report_is_aggregate_only_and_incomplete_identity_is_not_zero(self) -> None:
        surface = summary()
        surface["distinct_defined_tools"] = None
        surface["distinct_used_tools"] = None
        surface["unused_tools_lower_bound"] = None
        public = {
            "execution": {
                "sessions_completed": 10,
                "sessions_return_code_zero": 10,
                "sessions_return_code_nonzero": 0,
                "sessions_timed_out": 0,
            }
        }
        value = MODULE.build_report(
            public, MODULE.validate_summary(surface), 10, "codex-cli test"
        )
        self.assertFalse(value["shadow_gate"]["identity_evidence_available"])
        encoded = json.dumps(value, sort_keys=True)
        keys = object_keys(value)
        for forbidden in (
            "tool_name",
            "tool_name_hash",
            "schema_body",
            "arguments",
            "session_id",
            "request_id",
            "command_output",
        ):
            self.assertNotIn(forbidden, keys)
        self.assertNotIn("PRIVATE_CANARY", encoded)

    def test_unexpected_dto_field_is_rejected(self) -> None:
        surface = summary()
        surface["tool_name"] = "forbidden"
        with self.assertRaisesRegex(ValueError, "unexpected tool-surface summary contract"):
            MODULE.validate_summary(surface)


if __name__ == "__main__":
    unittest.main()
