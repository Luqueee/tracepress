#!/usr/bin/env python3
"""Prepare, but do not execute, a paired public Search quality pilot.

The pinned public repository and task definitions are the same as the Phase 4.5 shadow cohort.
Search output is consumed in memory to produce aggregate answer keys only. No provider session,
active assignment, command, path, source, or ToolResult body is persisted.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from typing import Any


RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
RIPGREP_URL = "https://github.com/BurntSushi/ripgrep"
REDUCER_ID = "search.result_projection"
REDUCER_VERSION = 1
EXPERIMENT_ID = "public-search-quality-pilot-001"
POLICY_VERSION = "search-quality-policy-v1"
ASSIGNMENT_PROBABILITY_BASIS_POINTS = 5_000


def checkout(repo: Path) -> None:
    if not (repo / ".git").exists():
        repo.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["git", "clone", "--filter=blob:none", RIPGREP_URL, str(repo)],
            check=True,
            capture_output=True,
            text=True,
        )
    subprocess.run(
        ["git", "-C", str(repo), "checkout", "--detach", RIPGREP_SHA],
        check=True,
        capture_output=True,
        text=True,
    )


def search_tasks() -> list[dict[str, str]]:
    # Query strings remain execution-only and are deliberately not included in the report.
    patterns = ["fn main", "Result<", "TODO|FIXME", "struct ", "trait ", "Cargo.toml"]
    return [
        {"task_id": f"search-{index:02}", "pattern": pattern}
        for index, pattern in enumerate(patterns, start=1)
    ]


def run_search(repo: Path, pattern: str, timeout: int) -> tuple[int, bytes]:
    result = subprocess.run(
        ["rg", "--line-number", "--no-heading", "--color", "never", pattern, "--glob", "*.rs"],
        cwd=repo,
        capture_output=True,
        timeout=timeout,
    )
    # Keep bounded output in memory only; callers retain aggregate counts and discard bytes.
    return result.returncode, (result.stdout + result.stderr)[:1_048_577]


def answer_key(return_code: int, output: bytes) -> dict[str, int]:
    lines = [line for line in output.decode("utf-8", errors="replace").splitlines() if line]
    files = {line.split(":", 1)[0] for line in lines if ":" in line}
    return {
        "expected_exit_code": return_code,
        "expected_match_count": len(lines),
        "expected_unique_file_count": len(files),
    }


def seed_for(task_id: str, arm: str) -> int:
    digest = hashlib.sha256(f"{EXPERIMENT_ID}:{task_id}:{arm}".encode()).digest()
    return int.from_bytes(digest[:8], "big")


def pair_order_for(task_id: str) -> str:
    digest = hashlib.sha256(f"{EXPERIMENT_ID}:order:{task_id}".encode()).digest()
    return "control_first" if digest[0] % 2 == 0 else "treatment_first"


def assignment(task_id: str, arm: str) -> dict[str, Any]:
    return {
        "experiment_id": EXPERIMENT_ID,
        "task_id": task_id,
        "session_id": f"planned-{EXPERIMENT_ID}-{task_id}-{arm.lower()}",
        "arm": arm,
        "reducer_id": REDUCER_ID if arm == "Treatment" else None,
        "reducer_version": REDUCER_VERSION if arm == "Treatment" else None,
        "policy_version": POLICY_VERSION,
        "seed": seed_for(task_id, arm),
        "assignment_probability_basis_points": ASSIGNMENT_PROBABILITY_BASIS_POINTS,
        "planned_only": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workload-root",
        type=Path,
        default=Path("/tmp/tracepress-public-workloads-001"),
    )
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    repo = args.workload_root.resolve() / "ripgrep"
    checkout(repo)
    task_rows: list[dict[str, Any]] = []
    for task in search_tasks():
        return_code, output = run_search(repo, task["pattern"], args.timeout)
        task_rows.append(
            {
                "task_id": task["task_id"],
                "workload_family": "search",
                "repository": "BurntSushi/ripgrep",
                "commit_sha": RIPGREP_SHA,
                "evaluator": "KnownAnswer",
                "timeout_ms": args.timeout * 1_000,
                "expected_artifact": "QueryResult",
                "answer_key": answer_key(return_code, output),
                "ground_truth_content_persisted": False,
                "paired_order": pair_order_for(task["task_id"]),
            }
        )

    assignments = [
        assignment(task["task_id"], arm)
        for task in task_rows
        for arm in ("Control", "Treatment")
    ]
    report = {
        "experiment_id": EXPERIMENT_ID,
        "status": "prepared_not_started",
        "phase": "4.5",
        "family": "Search",
        "reducer": {"id": REDUCER_ID, "version": REDUCER_VERSION},
        "policy_version": POLICY_VERSION,
        "shadow_only": True,
        "active_started": False,
        "provider_requests": 0,
        "sessions": 0,
        "forwarding_mutations": 0,
        "shadow_jobs_admitted": 0,
        "shadow_jobs_processed": 0,
        "shadow_job_drops": 0,
        "recovery_failures": 0,
        "determinism_failures": 0,
        "quality_evaluator_ready": True,
        "assignment": {
            "unit": "session",
            "arms": ["Control", "Treatment"],
            "probability_basis_points": ASSIGNMENT_PROBABILITY_BASIS_POINTS,
            "paired_tasks": len(task_rows),
            "planned_assignments": len(assignments),
        },
        "repository_pins": [
            {"repository": "BurntSushi/ripgrep", "url": RIPGREP_URL, "commit_sha": RIPGREP_SHA}
        ],
        "tasks": task_rows,
        "assignments": assignments,
        "limitations": [
            "This artifact prepares an objective answer key; no provider session was started.",
            "Commands, arguments, paths, source, and ToolResult bytes remained transient only.",
            "Provider usage, cache behavior, recovery frequency, and task quality are unmeasured.",
            "The next run must use the same pinned tasks with fail-open treatment handling.",
        ],
    }
    args.output_json.parent.mkdir(parents=True, exist_ok=True)
    args.output_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    lines = [
        f"# {EXPERIMENT_ID}",
        "",
        "Status: **prepared, not started**. This is a metadata-only preparation artifact; no active Control/Treatment session ran.",
        "",
        f"Reducer: `{REDUCER_ID}` v{REDUCER_VERSION}. Assignment unit: **session**. Planned pairs: **{len(task_rows)}**.",
        "",
        "## Pinned public workload",
        "",
        f"- `BurntSushi/ripgrep` at `{RIPGREP_SHA}`",
        "- Commands, arguments, paths, source, and ToolResult bytes were execution-only.",
        "",
        "## Objective answer keys",
        "",
        "| Task | Order | Exit | Matches | Unique files | Evaluator |",
        "|---|---|---:|---:|---:|---|",
    ]
    for row in task_rows:
        key = row["answer_key"]
        lines.append(
            f"| `{row['task_id']}` | `{row['paired_order']}` | {key['expected_exit_code']} | "
            f"{key['expected_match_count']} | {key['expected_unique_file_count']} | `{row['evaluator']}` |"
        )
    lines += [
        "",
        "## Pilot gate",
        "",
        "The Search shadow candidate passed the Phase 4.5 opportunity gate, so this manifest prepares a future paired quality pilot. It does not authorize or execute active request rewriting.",
        "",
        "Before starting, the runner must persist session-level Control/Treatment assignments, keep treatment fail-open, collect objective outcomes, and compare provider cached/uncached input. No economic or universal quality claim follows from this preparation artifact.",
        "",
        "Privacy audit: **no raw content persisted**. Forwarding mutations: **0**. Active sessions: **0**.",
    ]
    args.output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
