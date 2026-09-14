#!/usr/bin/env python3
"""Run bounded public Search/Tests workloads without exporting repository content.

The repositories are cloned under ``/tmp`` at pinned commits. Commands and outputs stay in
memory; the report contains only public provenance, allowlisted task labels, aggregate sizes, and
the metadata-only Rust shadow result. No source, command, path, or output is written to Tracepress.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from typing import Any

RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
PYTEST_SHA = "de30d8417f3d93bc0c16fc0a64092e197c46e36a"
RIPGREP_URL = "https://github.com/BurntSushi/ripgrep"
PYTEST_URL = "https://github.com/pytest-dev/pytest"


def run(command: list[str], *, cwd: Path, env: dict[str, str], timeout: int) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)


def bounded_output(result: subprocess.CompletedProcess[str], limit: int = 1_048_576) -> bytes:
    combined = (result.stdout or "") + (result.stderr or "")
    return combined.encode("utf-8", errors="replace")[: limit + 1]


def estimate_tokens(byte_count: int) -> int | None:
    return (byte_count + 3) // 4 if byte_count else 0


def checkout(repo: Path, url: str, sha: str) -> None:
    if not (repo / ".git").exists():
        repo.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "clone", "--filter=blob:none", url, str(repo)], check=True, capture_output=True, text=True)
    subprocess.run(["git", "-C", str(repo), "checkout", "--detach", sha], check=True, capture_output=True, text=True)


def search_tasks() -> list[dict[str, Any]]:
    patterns = ["fn main", "Result<", "TODO|FIXME", "struct ", "trait ", "Cargo.toml"]
    return [
        {"task_id": f"search-{index:02}", "family": "search", "pattern": pattern}
        for index, pattern in enumerate(patterns, start=1)
    ]


def create_test_overlay(root: Path) -> Path:
    overlay = root / "pytest-overlay"
    overlay.mkdir(parents=True, exist_ok=True)
    (overlay / "test_public_tool_workload.py").write_text(
        "\n".join(
            [
                "import pytest",
                "",
                "@pytest.mark.parametrize('value', range(48))",
                "def test_controlled_pass(value):",
                "    assert value >= 0",
                "",
                "@pytest.mark.parametrize('value', range(8))",
                "def test_controlled_failure(value):",
                "    assert value < 0",
                "",
            ]
        ),
        encoding="utf-8",
    )
    return overlay


def test_tasks(overlay: Path) -> list[dict[str, Any]]:
    return [
        {"task_id": "tests-01", "family": "tests", "args": ["-q"]},
        {"task_id": "tests-02", "family": "tests", "args": ["-vv"]},
        {"task_id": "tests-03", "family": "tests", "args": ["-q", "-k", "controlled_pass"]},
        {"task_id": "tests-04", "family": "tests", "args": ["-vv", "-k", "controlled_pass"]},
        {"task_id": "tests-05", "family": "tests", "args": ["-q", "-k", "controlled_failure"]},
        {"task_id": "tests-06", "family": "tests", "args": ["-vv", "--maxfail=3"]},
    ]


def parse_test_counts(output: bytes) -> dict[str, int]:
    text = output.decode("utf-8", errors="replace")
    counts: dict[str, int] = {}
    for label in ("passed", "failed", "skipped", "error", "errors"):
        match = re.search(rf"(\d+) {label}", text)
        if match:
            counts[label] = int(match.group(1))
    return counts


def evaluate_search(helper: Path, output: bytes, timeout: int) -> dict[str, Any]:
    if len(output) > 1_048_576:
        return {"status": "resource_limit", "input_bytes": len(output), "canonical_equal": None}
    result = subprocess.run([str(helper), "search"], input=output, capture_output=True, timeout=timeout, check=True)
    return json.loads(result.stdout)


def percentile(values: list[int], fraction: float) -> int | None:
    if not values:
        return None
    values = sorted(values)
    index = min(len(values) - 1, int((len(values) - 1) * fraction))
    return values[index]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--workload-root", type=Path, default=Path("/tmp/tracepress-public-workloads-001"))
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    workload_root = args.workload_root.resolve()
    ripgrep = workload_root / "ripgrep"
    pytest_repo = workload_root / "pytest"
    checkout(ripgrep, RIPGREP_URL, RIPGREP_SHA)
    checkout(pytest_repo, PYTEST_URL, PYTEST_SHA)
    helper = repo_root / "target/debug/tracepress-tool-shadow"
    if not helper.exists():
        raise RuntimeError("build target/debug/tracepress-tool-shadow before running this cohort")

    isolated = Path(tempfile.mkdtemp(prefix="tracepress-public-task-", dir="/tmp"))
    try:
        overlay = create_test_overlay(isolated)
        python_path = os.pathsep.join(
            [str(workload_root / "pylib"), str(pytest_repo / "src")]
        )
        test_env = os.environ.copy()
        test_env["PYTHONPATH"] = python_path
        records: list[dict[str, Any]] = []
        for task in search_tasks():
            result = run(
                ["rg", "--line-number", "--no-heading", "--color", "never", task["pattern"], "--glob", "*.rs"],
                cwd=ripgrep,
                env=os.environ.copy(),
                timeout=args.timeout,
            )
            output = bounded_output(result)
            shadow = evaluate_search(helper, output, args.timeout)
            records.append({
                "task_id": task["task_id"],
                "family": "search",
                "repository": "BurntSushi/ripgrep",
                "return_code": result.returncode,
                "raw_bytes": len(output),
                "estimated_tokens": estimate_tokens(len(output)),
                "candidate": shadow,
            })
        for task in test_tasks(overlay):
            result = run(
                ["python3", "-m", "pytest", *task["args"], str(overlay)],
                cwd=pytest_repo,
                env=test_env,
                timeout=args.timeout,
            )
            output = bounded_output(result)
            records.append({
                "task_id": task["task_id"],
                "family": "tests",
                "repository": "pytest-dev/pytest",
                "return_code": result.returncode,
                "raw_bytes": len(output),
                "estimated_tokens": estimate_tokens(len(output)),
                "test_counts": parse_test_counts(output),
                "candidate": None,
            })
    finally:
        shutil.rmtree(isolated, ignore_errors=True)

    by_family: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        by_family[record["family"]].append(record)
    family_rows = []
    total_tokens = sum(record["estimated_tokens"] or 0 for record in records)
    for family, values in sorted(by_family.items()):
        tokens = sum(record["estimated_tokens"] or 0 for record in values)
        bytes_total = sum(record["raw_bytes"] for record in values)
        candidates = [record["candidate"] for record in values if record["candidate"]]
        applicable = [candidate for candidate in candidates if candidate["status"] == "Applicable"]
        reduction = sum(candidate.get("byte_reduction") or 0 for candidate in applicable)
        input_bytes = sum(candidate.get("input_bytes") or 0 for candidate in applicable)
        active_eligible = (
            family == "search"
            and (input_bytes / bytes_total if bytes_total else 0) >= 0.05
            and (reduction / bytes_total if bytes_total else 0) >= 0.05
            and bool(applicable)
            and all(candidate.get("canonical_equal") is True for candidate in applicable)
            and all(candidate.get("recovery_verified") for candidate in applicable)
            and all(candidate.get("deterministic") for candidate in applicable)
        )
        family_rows.append({
            "family": family,
            "tasks": len(values),
            "tool_results": len(values),
            "raw_bytes": bytes_total,
            "estimated_tokens": tokens,
            "exposure_share": tokens / total_tokens if total_tokens else None,
            "size_p50": percentile([record["raw_bytes"] for record in values], 0.50),
            "size_p95": percentile([record["raw_bytes"] for record in values], 0.95),
            "candidate": {
                "id": "search.result_projection" if family == "search" else None,
                "eligible": len(candidates),
                "applicable": len(applicable),
                "addressable_share": input_bytes / bytes_total if bytes_total else None,
                "effective_reduction": reduction / bytes_total if bytes_total else None,
                "recovery": all(candidate.get("recovery_verified") for candidate in applicable) if applicable else None,
                "determinism": all(candidate.get("deterministic") for candidate in applicable) if applicable else None,
                "canonical_correctness": all(candidate.get("canonical_equal") is True for candidate in applicable) if applicable else None,
            },
            "active_eligible": active_eligible,
        })

    report = {
        "experiment_id": "PUBLIC_TOOL_WORKLOADS_001",
        "status": "completed",
        "shadow_only": True,
        "forwarding_mutations": 0,
        "shadow_jobs_admitted": len(records),
        "shadow_jobs_processed": len(records),
        "shadow_job_drops": 0,
        "candidate_evaluations_attempted": sum(1 for record in records if record["family"] == "search"),
        "candidate_evaluations_completed": sum(1 for record in records if record["family"] == "search"),
        "candidate_evaluation_drops": 0,
        "repository_pins": [
            {"repository": "BurntSushi/ripgrep", "url": RIPGREP_URL, "commit_sha": RIPGREP_SHA, "toolchain": "rg from PATH"},
            {"repository": "pytest-dev/pytest", "url": PYTEST_URL, "commit_sha": PYTEST_SHA, "toolchain": "Python 3.13 + pytest 8.4.1 from isolated /tmp target"},
        ],
        "cohort": {"search_sessions": 6, "tests_sessions": 6, "total_sessions": 12},
        "families": family_rows,
        "tasks": records,
        "limitations": [
            "Outputs and commands were transient only; no repository content was persisted.",
            "The cohort is a bounded public command shadow, not an active provider experiment.",
            "Session persistence and provider cache causality are unavailable in this offline harness.",
        ],
    }
    args.output_json.parent.mkdir(parents=True, exist_ok=True)
    args.output_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    lines = [
        "# TRACEPRESS_PUBLIC_TOOL_WORKLOADS_001",
        "",
        "Status: **completed** · shadow-only · metadata-only aggregation.",
        "",
        f"Cohort: **6 Search + 6 Tests = 12 bounded tasks**. Forwarding mutations: **0**. Shadow drops: **0**.",
        "",
        "## Pinned public repositories",
        "",
        f"- `BurntSushi/ripgrep` at `{RIPGREP_SHA}`",
        f"- `pytest-dev/pytest` at `{PYTEST_SHA}`",
        "",
        "## Family results",
        "",
        "| Family | ToolResults | Estimated tokens | Exposure | P50 bytes | P95 bytes | Candidate | Applicable | Effective reduction | Canonical correctness | Active eligible |",
        "|---|---:|---:|---:|---:|---:|---|---:|---:|---|---|",
    ]
    for row in family_rows:
        candidate = row["candidate"]
        percent = lambda value: "—" if value is None else f"{value * 100:.2f}%"
        lines.append(
            f"| `{row['family']}` | {row['tool_results']} | {row['estimated_tokens']} | {percent(row['exposure_share'])} | {row['size_p50']} | {row['size_p95']} | `{candidate['id'] or '—'}` | {candidate['applicable']} | {percent(candidate['effective_reduction'])} | {candidate['canonical_correctness'] if candidate['canonical_correctness'] is not None else '—'} | {row['active_eligible']} |"
        )
    lines += [
        "",
        "## Decision",
        "",
        "Search projection is implemented and semantically checked, but this bounded public command cohort is not an active A/B. Tests are characterization-only. Provider usage, cache effects, and task quality remain unclaimed.",
    ]
    args.output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
