#!/usr/bin/env python3
"""Run the bounded public Search Control/Treatment quality pilot.

This is the first active experiment for ``search.result_projection``. It uses only the pinned
public ripgrep checkout and the metadata-only preparation manifest. Prompts, commands, paths,
source, ToolResult bytes, and model output remain transient; the report stores aggregate quality,
provider, and active-rewrite counters only.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import tempfile
import time
from typing import Any


EXPERIMENT_ID = "public-search-quality-pilot-001"
REDUCER_ID = "search.result_projection"
REDUCER_VERSION = 1
MODEL = "gpt-5.6-luna"
RESPONSE_RE = re.compile(r"\{[^{}]{0,512}\}")
COUNTER_RE = re.compile(r"([a-zA-Z0-9_]+)=([0-9]+)")
SEARCH_PATTERNS = {
    "search-01": "fn main",
    "search-02": "Result<",
    "search-03": "TODO|FIXME",
    "search-04": "struct ",
    "search-05": "trait ",
    "search-06": "Cargo.toml",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument(
        "--workload-root", type=Path, default=Path("/tmp/tracepress-public-workloads-001")
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path(
            "reports/public-tool-workloads-001/"
            "TRACEPRESS_SEARCH_QUALITY_PILOT_PREPARATION_001.json"
        ),
    )
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=180)
    return parser.parse_args()


def load_manifest(path: Path) -> dict[str, Any]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("status") != "prepared_not_started":
        raise RuntimeError("quality pilot manifest is not in prepared_not_started state")
    if document.get("reducer", {}).get("id") != REDUCER_ID:
        raise RuntimeError("quality pilot manifest does not target the expected reducer")
    return document


def prompt_for(task_id: str, pattern: str) -> str:
    # The task query is execution-only and is never copied into the metadata report.
    import shlex

    return (
        f"Complete public search task {task_id}. Use the shell exactly once in the current "
        "repository. Run rg --line-number --no-heading --color never "
        f"{shlex.quote(pattern)} --glob '*.rs'. Then reply with one JSON object only, with "
        "integer fields matches and files, where matches is the number of result lines and "
        "files is the number of unique files. Do not include any other text."
    )


def parse_answer(stdout: str) -> dict[str, int] | None:
    # Only the two objective aggregate fields cross the evaluator boundary.
    for candidate in reversed(RESPONSE_RE.findall(stdout)):
        try:
            value = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if (
            isinstance(value, dict)
            and isinstance(value.get("matches"), int)
            and isinstance(value.get("files"), int)
            and value["matches"] >= 0
            and value["files"] >= 0
        ):
            return {"matches": value["matches"], "files": value["files"]}
    return None


def counters(stdout: str) -> dict[str, int]:
    result: dict[str, int] = {}
    for line in stdout.splitlines():
        for key, value in COUNTER_RE.findall(line):
            if key.startswith(("active_compression_", "analysis_", "context_")):
                result[key] = int(value)
    return result


def scalar(connection: sqlite3.Connection, query: str) -> int:
    value = connection.execute(query).fetchone()[0]
    return int(value or 0)


def snapshot(connection: sqlite3.Connection) -> dict[str, int | None]:
    row = connection.execute(
        "SELECT SUM(input_total), SUM(input_uncached), SUM(cache_read), "
        "SUM(output_total), SUM(reasoning) FROM provider_usage"
    ).fetchone()
    return {
        "provider_requests": scalar(connection, "SELECT COUNT(*) FROM provider_requests"),
        "provider_errors": scalar(
            connection,
            "SELECT COUNT(*) FROM provider_attempts "
            "WHERE status <> 'completed' OR error_code IS NOT NULL OR transport_error IS NOT NULL",
        ),
        "context_snapshots": scalar(connection, "SELECT COUNT(*) FROM context_snapshots"),
        "context_complete": scalar(
            connection, "SELECT COUNT(*) FROM context_snapshots WHERE status = 'complete'"
        ),
        "tool_calls": scalar(connection, "SELECT COALESCE(SUM(tool_count), 0) FROM provider_requests"),
        "input_total": row[0],
        "input_uncached": row[1],
        "cache_read": row[2],
        "output_total": row[3],
        "reasoning": row[4],
    }


def delta(after: dict[str, int | None], before: dict[str, int | None]) -> dict[str, int | None]:
    result: dict[str, int | None] = {}
    for key, value in after.items():
        previous = before.get(key)
        result[key] = None if value is None or previous is None else value - previous
    return result


def run_arm(
    repo_root: Path,
    workload_root: Path,
    tasks: list[dict[str, Any]],
    arm: str,
    timeout: int,
) -> dict[str, Any]:
    public_repo = workload_root / "ripgrep"
    cli = repo_root / "target/debug/tracepress"
    daemon = repo_root / "target/debug/tracepressd"
    if not cli.exists() or not daemon.exists():
        raise RuntimeError("build target/debug/tracepress and target/debug/tracepressd first")
    root = Path(tempfile.mkdtemp(prefix=f"tracepress-search-quality-{arm}-", dir="/tmp"))
    environment = os.environ.copy()
    environment.update(
        {
            "TRACEPRESS_HOME": str(root),
            "TRACEPRESS_CONTEXT_ANALYSIS": "shadow",
            "TRACEPRESS_SHADOW_COMPRESSION": "off",
            "TRACEPRESS_ACTIVE_COMPRESSION": (
                REDUCER_ID if arm == "treatment" else "off"
            ),
            "TRACEPRESS_MEASUREMENT_RUN_ID": f"{EXPERIMENT_ID}-{arm}",
        }
    )
    daemon_environment = dict(environment)
    daemon_environment.update(
        {
            "TRACEPRESS_DATABASE": str(root / "tracepress.sqlite3"),
            "TRACEPRESS_CONTROL_SOCKET": str(root / "tracepress.sock"),
            "TRACEPRESS_CONTROL_CREDENTIAL": str(root / "control.cred"),
            "TRACEPRESS_DAEMON_READY": str(root / "daemon.ready"),
        }
    )
    process: subprocess.Popen[str] | None = None
    daemon_log = None
    try:
        subprocess.run(
            [str(cli), "init"],
            cwd=public_repo,
            env=environment,
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
        daemon_log = (root / "daemon.log").open("w", encoding="utf-8")
        process = subprocess.Popen(
            [str(daemon)],
            cwd=repo_root,
            env=daemon_environment,
            stdout=daemon_log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            text=True,
        )
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f"{arm} daemon exited before readiness")
            if (root / "daemon.ready").exists():
                status = subprocess.run(
                    [str(cli), "daemon", "status"],
                    cwd=public_repo,
                    env=environment,
                    capture_output=True,
                    text=True,
                    timeout=30,
                )
                if status.returncode == 0:
                    break
            time.sleep(0.1)
        else:
            raise RuntimeError(f"{arm} daemon did not become ready")

        rows: list[dict[str, Any]] = []
        with sqlite3.connect(f"file:{root / 'tracepress.sqlite3'}?mode=ro", uri=True) as connection:
            previous = snapshot(connection)
        for task in tasks:
            try:
                completed = subprocess.run(
                    [
                        str(cli),
                        "run",
                        "codex",
                        "exec",
                        "-m",
                        MODEL,
                        "-s",
                        "read-only",
                        "--skip-git-repo-check",
                        prompt_for(task["task_id"], task["pattern"]),
                    ],
                    cwd=public_repo,
                    env=environment,
                    capture_output=True,
                    text=True,
                    timeout=timeout,
                )
                timed_out = False
            except subprocess.TimeoutExpired:
                # A timeout is an objective evaluator-unavailable outcome, not a zero result.
                completed = subprocess.CompletedProcess([], 124, "", "")
                timed_out = True
            with sqlite3.connect(
                f"file:{root / 'tracepress.sqlite3'}?mode=ro", uri=True
            ) as connection:
                current = snapshot(connection)
            observed = parse_answer(completed.stdout)
            expected = task["answer_key"]
            counters_row = counters(completed.stdout)
            expected_answer = {
                "matches": expected["expected_match_count"],
                "files": expected["expected_unique_file_count"],
            }
            task_success = completed.returncode == 0 and observed == expected_answer
            rows.append(
                {
                    "task_id": task["task_id"],
                    "arm": arm,
                    "return_code": completed.returncode,
                    "timed_out": timed_out,
                    "evaluator": "KnownAnswer",
                    "evaluator_status": "unavailable"
                    if timed_out or observed is None
                    else "success" if task_success else "failure",
                    "task_success": task_success,
                    "answer_observed": observed,
                    "provider": delta(current, previous),
                    "active": {
                        key: counters_row.get(key, 0)
                        for key in (
                            "active_compression_attempts",
                            "active_compression_rewrites",
                            "active_compression_evaluated_spans",
                            "active_compression_evaluated_input_bytes",
                            "active_compression_evaluated_candidate_bytes",
                            "active_compression_no_improvement",
                            "active_compression_not_applicable",
                            "active_compression_recovery_failures",
                            "active_compression_determinism_failures",
                            "active_compression_resource_limits",
                            "active_compression_internal_errors",
                        )
                    },
                }
            )
            previous = current
        with sqlite3.connect(f"file:{root / 'tracepress.sqlite3'}?mode=ro", uri=True) as connection:
            aggregate = snapshot(connection)
        return {"arm": arm, "rows": rows, "aggregate": aggregate}
    finally:
        if process is not None:
            subprocess.run(
                [str(cli), "daemon", "stop"],
                cwd=public_repo,
                env=environment,
                capture_output=True,
                text=True,
                timeout=30,
            )
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
        if daemon_log is not None:
            daemon_log.close()
        shutil.rmtree(root, ignore_errors=True)


def main() -> int:
    options = parse_args()
    manifest = load_manifest((options.repo_root / options.manifest).resolve())
    tasks = manifest["tasks"]
    repo_pins = manifest["repository_pins"]
    if not tasks or repo_pins[0]["repository"] != "BurntSushi/ripgrep":
        raise RuntimeError("unexpected public Search pilot manifest")
    execution_tasks = []
    for task in tasks:
        try:
            pattern = SEARCH_PATTERNS[task["task_id"]]
        except KeyError as error:
            raise RuntimeError("quality pilot manifest contains an unknown Search task") from error
        execution_tasks.append({**task, "pattern": pattern})
    control = run_arm(
        options.repo_root.resolve(),
        options.workload_root.resolve(),
        execution_tasks,
        "control",
        options.timeout,
    )
    treatment = run_arm(
        options.repo_root.resolve(),
        options.workload_root.resolve(),
        execution_tasks,
        "treatment",
        options.timeout,
    )
    all_rows = control["rows"] + treatment["rows"]
    active_evaluations = sum(
        row["active"]["active_compression_evaluated_spans"] for row in all_rows
    )
    active_rewrites = sum(row["active"]["active_compression_rewrites"] for row in all_rows)
    report = {
        "experiment_id": EXPERIMENT_ID,
        "status": "completed" if active_rewrites > 0 else "completed_infrastructure_diagnostic",
        "phase": "4.5",
        "family": "Search",
        "reducer": {"id": REDUCER_ID, "version": REDUCER_VERSION},
        "repository_pins": repo_pins,
        "model": MODEL,
        "assignment_unit": "session",
        "shadow_only": False,
        "forwarding_mutations": sum(
            row["active"]["active_compression_rewrites"] for row in all_rows
        ),
        "active_candidate_evaluations": active_evaluations,
        "active_candidate_rewrites": active_rewrites,
        "treatment_exercised": active_rewrites > 0,
        "quality_comparable": active_rewrites > 0,
        "decision": (
            "blocked_no_active_candidate_applicability"
            if active_rewrites == 0
            else "pilot_quality_requires_follow_up"
        ),
        "recovery_failures": sum(
            row["active"]["active_compression_recovery_failures"] for row in all_rows
        ),
        "determinism_failures": sum(
            row["active"]["active_compression_determinism_failures"] for row in all_rows
        ),
        "arms": {
            "control": control,
            "treatment": treatment,
        },
        "tasks": all_rows,
        "limitations": [
            "This is a bounded public pilot, not a universal quality or provider-savings claim.",
            "Reports contain aggregate evaluator, provider, and active-rewrite metrics only.",
            "No prompt, command, path, source, ToolResult, response, or secret was persisted.",
            "The provider subscription billing model is not inferred from these observations.",
        ],
    }
    options.output_json.parent.mkdir(parents=True, exist_ok=True)
    options.output_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    rows = []
    for row in all_rows:
        provider = row["provider"]
        active = row["active"]
        rows.append(
            f"| `{row['arm']}` | `{row['task_id']}` | {str(row['task_success']).lower()} | "
            f"{provider['input_total'] if provider['input_total'] is not None else '—'} | "
            f"{provider['input_uncached'] if provider['input_uncached'] is not None else '—'} | "
            f"{active['active_compression_rewrites']} | {row['evaluator_status']} |"
        )
    markdown = "\n".join(
        [
            "# TRACEPRESS_SEARCH_QUALITY_ACTIVE_PILOT_001",
            "",
            "Bounded public Search Control/Treatment pilot using the pinned ripgrep workload.",
            "",
            f"Reducer: `{REDUCER_ID}` v{REDUCER_VERSION}. Model: `{MODEL}`. Sessions: `{len(all_rows)}`.",
            "",
            "| Arm | Task | Success | Input | Uncached | Rewrites | Evaluator |",
            "|---|---|---:|---:|---:|---:|---|",
            *rows,
            "",
            f"Forwarding mutations: **{report['forwarding_mutations']}**. Recovery failures: **{report['recovery_failures']}**. Determinism failures: **{report['determinism_failures']}**.",
            "",
            f"Active candidate evaluations: **{report['active_candidate_evaluations']}**; active rewrites: **{report['active_candidate_rewrites']}**.",
            "Treatment quality is not comparable when the candidate was never exercised; this run is an infrastructure diagnostic, not an efficacy result.",
            "This pilot is limited to the pinned public workload. It does not establish universal quality, provider savings, cache causality, or economic impact.",
            "",
            "Privacy audit: **metadata-only report**; no raw task or provider content persisted.",
        ]
    )
    options.output_md.write_text(markdown + "\n", encoding="utf-8")
    print(markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
