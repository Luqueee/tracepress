#!/usr/bin/env python3
"""Run the controlled public Phase 6.1 Tool Surface Shadow cohort.

The existing public Search workload owns provider execution and temporary database
capture. This driver starts the real read-only Observatory against that database and
publishes only the allowlisted ``/api/v1/tool-surface`` aggregate DTO. No tool is
removed, no provider request is modified, and the temporary database is deleted.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
from typing import Any
from urllib.error import URLError
from urllib.request import urlopen


EXPERIMENT_ID = "tool-surface-shadow-pilot-014"
RIPGREP_REPOSITORY = "BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"
SUMMARY_KEYS = {
    "mode",
    "provider_effect_active",
    "requests_observed",
    "schema_observations",
    "schema_observation_coverage_basis_points",
    "tool_definitions_exposed",
    "schema_bytes",
    "estimated_schema_tokens",
    "repeated_schema_tokens",
    "repeated_schema_share_basis_points",
    "tool_calls_observed",
    "distinct_defined_tools",
    "distinct_used_tools",
    "unused_tools_lower_bound",
    "provider_usage",
}
METRIC_KEYS = {"value", "source"}
USAGE_KEYS = {
    "input_tokens",
    "cached_input_tokens",
    "uncached_input_tokens",
    "cache_ratio",
    "output_tokens",
    "reasoning_tokens",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--sessions", type=int, default=10, choices=range(1, 11))
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    return parser.parse_args()


def assert_metric(metric: object, field: str) -> None:
    if not isinstance(metric, dict) or set(metric) != METRIC_KEYS:
        raise ValueError(f"unexpected metric contract for {field}")
    if metric["source"] not in {"provider_reported", "locally_estimated", "unavailable"}:
        raise ValueError(f"unexpected metric source for {field}")
    if metric["value"] is not None and not isinstance(metric["value"], (int, float)):
        raise ValueError(f"unexpected metric value for {field}")


def validate_summary(summary: object) -> dict[str, Any]:
    if not isinstance(summary, dict) or set(summary) != SUMMARY_KEYS:
        raise ValueError("unexpected tool-surface summary contract")
    for field in (
        "tool_definitions_exposed",
        "schema_bytes",
        "estimated_schema_tokens",
        "repeated_schema_tokens",
    ):
        assert_metric(summary[field], field)
    usage = summary["provider_usage"]
    if not isinstance(usage, dict) or set(usage) != USAGE_KEYS:
        raise ValueError("unexpected provider-usage contract")
    for field, metric in usage.items():
        assert_metric(metric, f"provider_usage.{field}")
    if summary["mode"] != "shadow" or summary["provider_effect_active"] is not False:
        raise ValueError("Tool Surface pilot requires non-mutating Shadow evidence")
    return summary


def choose_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def fetch_tool_surface(cli: Path, state_root: Path, repo_root: Path) -> dict[str, Any]:
    port = choose_loopback_port()
    environment = os.environ.copy()
    environment.update(
        {
            "TRACEPRESS_HOME": str(state_root),
            "TRACEPRESS_REPORTS": str(repo_root / "reports"),
        }
    )
    log_path = state_root / "observatory.log"
    with log_path.open("w", encoding="utf-8") as log:
        process = subprocess.Popen(
            [str(cli), "ui", "--port", str(port)],
            cwd=repo_root,
            env=environment,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            text=True,
        )
        try:
            deadline = time.monotonic() + 20
            url = f"http://127.0.0.1:{port}/api/v1/tool-surface"
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError("Observatory exited before Tool Surface became available")
                try:
                    with urlopen(url, timeout=1) as response:  # noqa: S310 - fixed loopback URL
                        return validate_summary(json.load(response))
                except (URLError, TimeoutError, ConnectionError, json.JSONDecodeError):
                    time.sleep(0.1)
            raise RuntimeError("Observatory Tool Surface endpoint did not become ready")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)


def metric_value(summary: dict[str, Any], field: str) -> int | float | None:
    value = summary[field]["value"]
    return value if isinstance(value, (int, float)) else None


def codex_version() -> str:
    result = subprocess.run(
        ["codex", "--version"],
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
    )
    version = result.stdout.strip()
    if not version or len(version) > 128 or "\n" in version:
        raise ValueError("unexpected Codex version output")
    return version


def build_report(
    public_execution: dict[str, Any],
    summary: dict[str, Any],
    sessions_requested: int,
    codex_version_value: str,
) -> dict[str, Any]:
    execution = public_execution.get("execution", {})
    completed = execution.get("sessions_completed") == sessions_requested
    successful = execution.get("sessions_return_code_zero") == sessions_requested
    schema_coverage = summary["schema_observation_coverage_basis_points"]
    exposure_observed = all(
        (metric_value(summary, field) or 0) > 0
        for field in ("tool_definitions_exposed", "schema_bytes", "estimated_schema_tokens")
    )
    identity_evidence_available = all(
        summary[field] is not None
        for field in ("distinct_defined_tools", "distinct_used_tools", "unused_tools_lower_bound")
    )
    provider_usage_available = summary["provider_usage"]["input_tokens"]["value"] is not None
    measurement_complete = (
        completed
        and successful
        and schema_coverage == 10_000
        and provider_usage_available
    )
    if measurement_complete and exposure_observed:
        decision = "shadow_evidence_ready_for_policy_design"
    elif measurement_complete:
        decision = "reject_active_tool_selection_no_observed_schema_surface"
    else:
        decision = "insufficient_shadow_evidence"
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "6.1",
        "status": "completed" if completed else "completed_with_execution_gaps",
        "mode": "shadow",
        "workspace_class": "public_controlled",
        "repository_pin": {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA},
        "model": MODEL,
        "codex_version": codex_version_value,
        "execution": {
            "sessions_requested": sessions_requested,
            "sessions_completed": execution.get("sessions_completed", 0),
            "sessions_return_code_zero": execution.get("sessions_return_code_zero", 0),
            "sessions_return_code_nonzero": execution.get("sessions_return_code_nonzero", 0),
            "sessions_timed_out": execution.get("sessions_timed_out", 0),
        },
        "tool_surface": summary,
        "shadow_gate": {
            "schema_coverage_complete": schema_coverage == 10_000,
            "material_schema_exposure_observed": exposure_observed,
            "identity_evidence_available": identity_evidence_available,
            "provider_usage_available": provider_usage_available,
            "provider_effect_active": False,
            "tool_selection_active": False,
            "decision": decision,
        },
        "privacy": {
            "report_contract": "aggregate_allowlist_only",
            "raw_content_persisted": False,
            "paths_persisted": False,
            "commands_persisted": False,
            "responses_persisted": False,
            "tool_names_persisted": False,
            "tool_identity_material_persisted": False,
        },
        "limitations": [
            "Shadow mode does not remove, rewrite, reorder, or select tools.",
            "Provider usage is observational and does not establish causal savings.",
            "The bounded public Search workload does not establish general tool-surface behavior.",
            "An active experiment requires objective task-quality and trajectory evaluation.",
        ],
    }


def display(value: object) -> str:
    return "—" if value is None else str(value)


def markdown(report: dict[str, Any]) -> str:
    surface = report["tool_surface"]
    usage = surface["provider_usage"]
    gate = report["shadow_gate"]
    lines = [
        "# TRACEPRESS_TOOL_SURFACE_SHADOW_PILOT_014",
        "",
        "Phase 6.1 controlled public Tool Surface characterization. Shadow only; no tool or provider request was modified.",
        "",
        f"Runtime: **{report['codex_version']}**. Model: **{report['model']}**.",
        "",
        f"Sessions: **{report['execution']['sessions_completed']}/{report['execution']['sessions_requested']}**. Decision: **{gate['decision']}**.",
        "",
        "## Source exposure",
        "",
        "| Metric | Value |",
        "|---|---:|",
        f"| Requests observed | {surface['requests_observed']} |",
        f"| Complete schema observations | {surface['schema_observations']} |",
        f"| Schema coverage (basis points) | {display(surface['schema_observation_coverage_basis_points'])} |",
        f"| Tool definitions exposed | {display(surface['tool_definitions_exposed']['value'])} |",
        f"| Schema bytes | {display(surface['schema_bytes']['value'])} |",
        f"| Estimated schema tokens | {display(surface['estimated_schema_tokens']['value'])} |",
        f"| Repeated schema tokens | {display(surface['repeated_schema_tokens']['value'])} |",
        f"| Repeated schema share (basis points) | {display(surface['repeated_schema_share_basis_points'])} |",
        "",
        "## Observed usage",
        "",
        "| Metric | Value |",
        "|---|---:|",
        f"| Tool-call blocks | {surface['tool_calls_observed']} |",
        f"| Distinct definitions | {display(surface['distinct_defined_tools'])} |",
        f"| Distinct used tools | {display(surface['distinct_used_tools'])} |",
        f"| Unused lower bound | {display(surface['unused_tools_lower_bound'])} |",
        "",
        "## Provider usage (observed, not attributed)",
        "",
        "| Metric | Value |",
        "|---|---:|",
        f"| Input | {display(usage['input_tokens']['value'])} |",
        f"| Cached | {display(usage['cached_input_tokens']['value'])} |",
        f"| Uncached | {display(usage['uncached_input_tokens']['value'])} |",
        f"| Output | {display(usage['output_tokens']['value'])} |",
        f"| Reasoning | {display(usage['reasoning_tokens']['value'])} |",
        "",
        "Privacy: aggregate allowlist only; no prompt, command, path, source, output, response, tool name, or identity material is in this report.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    options = parse_args()
    repo_root = options.repo_root.resolve()
    cli = repo_root / "target/debug/tracepress"
    daemon = repo_root / "target/debug/tracepressd"
    workload = repo_root / "scripts/characterize_public_provider_native_search_001.py"
    if not cli.is_file() or not daemon.is_file():
        raise RuntimeError("build target/debug/tracepress and target/debug/tracepressd first")
    with tempfile.TemporaryDirectory(prefix="tracepress-tool-surface-014-", dir="/tmp") as raw:
        state_root = Path(raw)
        public_json = state_root / "public-execution.json"
        public_md = state_root / "public-execution.md"
        database = state_root / "tracepress.sqlite3"
        subprocess.run(
            [
                sys.executable,
                str(workload),
                "--repo-root",
                str(repo_root),
                "--sessions",
                str(options.sessions),
                "--timeout",
                str(options.timeout),
                "--metadata-database-output",
                str(database),
                "--output-json",
                str(public_json),
                "--output-md",
                str(public_md),
            ],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
            timeout=options.sessions * options.timeout + 180,
        )
        public_execution = json.loads(public_json.read_text(encoding="utf-8"))
        summary = fetch_tool_surface(cli, state_root, repo_root)
        report = build_report(public_execution, summary, options.sessions, codex_version())
    options.output_json.parent.mkdir(parents=True, exist_ok=True)
    options.output_json.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    rendered = markdown(report)
    options.output_md.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
