#!/usr/bin/env python3
"""Prepare, but never execute, Phase 5.0's paired Codex passthrough A/A cohort.

The manifest contains public repository provenance, objective outcome policy, and session-level
assignment only. Commands, prompts, paths, responses, ToolResults, and provider payloads stay
execution-only and cannot cross this report boundary.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from typing import Any


EXPERIMENT_ID = "source-passthrough-aa-001"
RIPGREP_URL = "https://github.com/BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
ARMS = ("Control", "Passthrough")


def checkout(repo: Path) -> None:
    if not (repo / ".git").exists():
        repo.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "clone", "--filter=blob:none", RIPGREP_URL, str(repo)], check=True, capture_output=True, text=True)
    subprocess.run(["git", "-C", str(repo), "checkout", "--detach", RIPGREP_SHA], check=True, capture_output=True, text=True)


def order(task_id: str) -> str:
    return "control_first" if hashlib.sha256(f"{EXPERIMENT_ID}:{task_id}".encode()).digest()[0] % 2 == 0 else "passthrough_first"


def assignments() -> list[dict[str, Any]]:
    rows = []
    for index in range(1, 11):
        task_id = f"cargo-test-{index:02}"
        for arm in ARMS:
            rows.append({
                "experiment_id": EXPERIMENT_ID,
                "task_id": task_id,
                "session_id": f"planned-{EXPERIMENT_ID}-{task_id}-{arm.lower()}",
                "arm": arm,
                "hook": "codex" if arm == "Passthrough" else "off",
                "reducer_id": "passthrough",
                "reducer_version": "v1",
                "paired_order": order(task_id),
                "planned_only": True,
            })
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workload-root", type=Path, default=Path("/tmp/tracepress-source-passthrough-aa-001"))
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    args = parser.parse_args()
    checkout(args.workload_root / "ripgrep")
    rows = assignments()
    report = {
        "experiment_id": EXPERIMENT_ID,
        "phase": "5.0",
        "status": "prepared_not_started",
        "repository_pin": {"repository": "BurntSushi/ripgrep", "url": RIPGREP_URL, "commit_sha": RIPGREP_SHA},
        "command_family": "cargo_test",
        "assignment": {"unit": "session", "arms": list(ARMS), "pairs": 10, "assignments": len(rows), "randomized_order": True},
        "treatment": {"hook": "codex", "reducer": "passthrough", "forwarding_mutation": False},
        "objective_outcome": {"kind": "command_exit", "expected_exit_status_class": "success"},
        "measurements": ["provider_input_total", "provider_cached_input", "provider_uncached_input", "provider_output", "provider_reasoning", "tool_calls", "command_retries", "duration", "source_execution_metadata"],
        "assignments": rows,
        "limitations": ["No Codex, provider, or tool session was started.", "No command, prompt, path, output, response, or ToolResult is persisted.", "This validates passthrough infrastructure only; it does not evaluate a reducer or savings."],
    }
    args.output_json.parent.mkdir(parents=True, exist_ok=True)
    args.output_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    lines = [f"# {EXPERIMENT_ID}", "", "Status: **prepared, not started**.", "", f"Pinned public repository: `BurntSushi/ripgrep` at `{RIPGREP_SHA}`.", "", "| Arm | Sessions | Hook | Reducer |", "|---|---:|---|---|", "| Control | 10 | off | passthrough |", "| Passthrough | 10 | codex | passthrough |", "", "The runner must use the same pinned task pairs, preserve session-level assignment, and report provider usage separately from source metadata. No filtering is enabled."]
    args.output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
