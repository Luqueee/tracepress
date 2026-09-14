#!/usr/bin/env python3
"""Run the naturalistic N=10 Phase 4.2.1 shadow cohort.

The temporary Tracepress home and SQLite database are deleted after aggregation. The report keeps
only bounded candidate metadata and aggregate provider observations; no prompt, tool result,
response, URL, header, fingerprint, or session identifier is written.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
from typing import Any


PROMPTS = [
    "Use one read-only shell command to list the top-level Tracepress directories, then summarize what each is for. Do not modify files.",
    "Use one bounded read-only search for TODO and FIXME in Rust files, then report only counts and paths. Do not modify files.",
    "Use one read-only command to list workspace Cargo.toml files, then summarize the crate boundaries. Do not modify files.",
    "Use one read-only command to report the current git status and explain whether the checkout is clean. Do not modify files.",
    "Use one read-only command to inspect the Observatory documentation headings and summarize its API areas. Do not modify files.",
    "Use one read-only command to inspect the compression module filenames and summarize their responsibilities. Do not modify files.",
    "Use one read-only command to list migration filenames and explain the additive schema history. Do not modify files.",
    "Use one read-only command to report Rust source file counts by crate. Do not modify files.",
    "Use one read-only command to print the current phase design document names and summarize the remaining gate. Do not modify files.",
    "Use one read-only command to report whether the dashboard API and WASM crates exist, then summarize the result. Do not modify files.",
]


def run(command: list[str], *, cwd: Path, env: dict[str, str], timeout: int) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)


def wait_for_daemon(cli: Path, root: Path, env: dict[str, str], process: subprocess.Popen[str]) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("tracepressd exited before readiness")
        if (root / "daemon.ready").exists() and run([str(cli), "daemon", "status"], cwd=root.parent, env=env, timeout=30).returncode == 0:
            return
        time.sleep(0.1)
    raise RuntimeError("tracepressd did not become ready")


def aggregate(database: Path) -> dict[str, Any]:
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    experiment = connection.execute(
        """SELECT experiment_id,status,forwarding_mutations,shadow_drops,
                  shadow_jobs_admitted,shadow_jobs_processed,shadow_job_drops,
                  candidate_evaluations_attempted,candidate_evaluations_completed,
                  candidate_evaluation_drops,recovery_failures,determinism_failures
           FROM compression_experiments WHERE experiment_id='shadow-pilot-003'"""
    ).fetchone()
    rows = connection.execute(
        """SELECT c.compressor_id, c.status, m.input_bytes, m.output_bytes, m.bytes_delta,
                  m.input_estimated_tokens, m.output_estimated_tokens, m.estimated_token_delta,
                  m.processing_us, m.recovery_verified, m.deterministic, c.provider_readability,
                  c.json_root_kind, c.json_array_length_bucket, c.json_object_key_count_bucket,
                  c.json_homogeneity_basis_points, c.json_primitive_cell_ratio_basis_points,
                  c.json_nested_cell_ratio_basis_points, c.text_shape
           FROM compression_candidates c
           JOIN compression_candidate_metrics m USING(candidate_id)
           WHERE c.experiment_id='shadow-pilot-003'"""
    ).fetchall()
    grouped: dict[str, list[sqlite3.Row]] = defaultdict(list)
    for row in rows:
        grouped[row["compressor_id"]].append(row)
    compressors = []
    for compressor, values in sorted(grouped.items()):
        applicable = [row for row in values if row["status"] == "applicable"]
        eligible_tokens = sum(row["input_estimated_tokens"] or 0 for row in values)
        applicable_tokens = sum(row["input_estimated_tokens"] or 0 for row in applicable)
        input_bytes = sum(row["input_bytes"] or 0 for row in applicable)
        reduction_bytes = sum(row["bytes_delta"] or 0 for row in applicable)
        input_tokens = sum(row["input_estimated_tokens"] or 0 for row in applicable)
        reduction_tokens = sum(row["estimated_token_delta"] or 0 for row in applicable)
        latencies = sorted(row["processing_us"] for row in values if row["processing_us"] is not None)
        p50 = latencies[(len(latencies) - 1) // 2] if latencies else None
        p95 = latencies[min(len(latencies) - 1, int(len(latencies) * 0.95))] if latencies else None
        recovery = [row for row in values if row["output_bytes"] is not None]
        readability = sorted({row["provider_readability"] or "unknown" for row in values})
        shapes = Counter(row["json_root_kind"] or row["text_shape"] or "unavailable" for row in values)
        compressors.append({
            "id": compressor,
            "provider_readability": readability,
            "eligible_blocks": len(values),
            "applicable_blocks": len(applicable),
            "applicability": (len(applicable) / len(values)) if values else None,
            "addressable_token_share": (applicable_tokens / eligible_tokens) if eligible_tokens else None,
            "input_bytes": input_bytes,
            "byte_reduction": reduction_bytes,
            "byte_reduction_ratio": (reduction_bytes / input_bytes) if input_bytes else None,
            "estimated_input_tokens": input_tokens,
            "estimated_token_reduction": reduction_tokens,
            "estimated_token_reduction_ratio": (reduction_tokens / input_tokens) if input_tokens else None,
            "recovery_success_rate": (sum(bool(row["recovery_verified"]) for row in recovery) / len(recovery)) if recovery else None,
            "determinism_success_rate": (sum(bool(row["deterministic"]) for row in recovery) / len(recovery)) if recovery else None,
            "processing_us": {"p50": p50, "p95": p95},
            "shape_distribution": dict(sorted(shapes.items())),
        })
    coverage_rows = connection.execute(
        """SELECT c.compressor_id,
                  CASE WHEN m.input_bytes < 1024 THEN '<1KiB'
                       WHEN m.input_bytes < 1048576 THEN '1KiB-1MiB'
                       ELSE '>=1MiB' END AS size_bucket,
                  COALESCE(b.detected_kind, 'unavailable') AS detected_kind,
                  COUNT(*) AS persisted_evaluations
           FROM compression_candidates c
           JOIN compression_candidate_metrics m USING(candidate_id)
           JOIN context_block_occurrences b USING(block_occurrence_id)
           WHERE c.experiment_id='shadow-pilot-003'
           GROUP BY c.compressor_id, size_bucket, detected_kind
           ORDER BY c.compressor_id, size_bucket, detected_kind"""
    ).fetchall()
    coverage = [dict(row) for row in coverage_rows]
    result = {
        "experiment": dict(experiment) if experiment else None,
        "sessions": connection.execute("SELECT COUNT(*) FROM sessions").fetchone()[0],
        "provider_requests": connection.execute("SELECT COUNT(*) FROM provider_requests").fetchone()[0],
        "analysis_snapshots": connection.execute("SELECT COUNT(*) FROM context_snapshots").fetchone()[0],
        "analysis_complete": connection.execute("SELECT COUNT(*) FROM context_snapshots WHERE status='complete'").fetchone()[0],
        "unknown_transformed": connection.execute("SELECT COUNT(*) FROM compression_candidates c JOIN context_block_occurrences b USING(block_occurrence_id) WHERE c.experiment_id='shadow-pilot-003' AND b.origin='unknown'").fetchone()[0],
        "candidate_coverage_by_size_and_kind": coverage,
        "candidate_evaluation_accounting": {
            "persisted_candidate_rows": len(rows),
            "counter_completed": (dict(experiment).get("candidate_evaluations_completed", 0) if experiment else 0),
            "counter_attempted": (dict(experiment).get("candidate_evaluations_attempted", 0) if experiment else 0),
            "counter_drops": (dict(experiment).get("candidate_evaluation_drops", 0) if experiment else 0),
        },
        "compressors": compressors,
    }
    connection.close()
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=90)
    args = parser.parse_args()
    repo = args.repo_root.resolve()
    cli = repo / "target/debug/tracepress"
    daemon = repo / "target/debug/tracepressd"
    root = Path(tempfile.mkdtemp(prefix="tracepress-shadow-pilot-003-"))
    env = os.environ.copy()
    env.update({
        "TRACEPRESS_HOME": str(root),
        "TRACEPRESS_CONTEXT_ANALYSIS": "shadow",
        "TRACEPRESS_SHADOW_COMPRESSION": "on",
        "TRACEPRESS_SHADOW_EXPERIMENT_ID": "shadow-pilot-003",
        "TRACEPRESS_DATABASE": str(root / "tracepress.sqlite3"),
        "TRACEPRESS_CONTROL_SOCKET": str(root / "tracepress.sock"),
        "TRACEPRESS_CONTROL_CREDENTIAL": str(root / "control.cred"),
        "TRACEPRESS_DAEMON_READY": str(root / "daemon.ready"),
    })
    daemon_process: subprocess.Popen[str] | None = None
    try:
        init = run([str(cli), "init"], cwd=repo, env=env, timeout=30)
        if init.returncode != 0:
            raise RuntimeError("tracepress init failed")
        log = (root / "daemon.log").open("w", encoding="utf-8")
        daemon_process = subprocess.Popen([str(daemon)], cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True, text=True)
        wait_for_daemon(cli, root, env, daemon_process)
        completed = []
        interrupted = False
        try:
            for index, prompt in enumerate(PROMPTS, start=1):
                try:
                    result = run([str(cli), "run", "codex", "exec", "-m", "gpt-5.6-luna", "-s", "read-only", "--skip-git-repo-check", prompt], cwd=repo, env=env, timeout=args.timeout)
                    completed.append({"index": index, "returncode": result.returncode})
                except subprocess.TimeoutExpired:
                    completed.append({"index": index, "returncode": None, "status": "timeout"})
                if completed[-1]["returncode"] not in (0,):
                    continue
        except KeyboardInterrupt:
            interrupted = True
        run([str(cli), "daemon", "stop"], cwd=repo, env=env, timeout=30)
        daemon_process.wait(timeout=30)
        result = aggregate(root / "tracepress.sqlite3")
        successful = sum(item["returncode"] == 0 for item in completed)
        shadow_drops = (result.get("experiment") or {}).get("shadow_drops", 0)
        status = "aborted_interrupted" if interrupted else ("completed" if successful == len(PROMPTS) and shadow_drops == 0 else "completed_degraded")
        result.update({"report_id": "TRACEPRESS_SHADOW_PILOT_003_N10", "status": status, "successful_invocations": successful, "invocations": completed, "privacy": "metadata_only"})
        args.output_json.parent.mkdir(parents=True, exist_ok=True)
        args.output_json.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        experiment_row = result.get("experiment") or {}
        accounting = result["candidate_evaluation_accounting"]
        lines = ["# Tracepress Shadow Pilot 003", "", f"Naturalistic N=10 shadow cohort; metadata-only aggregation. Status: **{result['status']}**.", "", f"Successful invocations: {result['successful_invocations']}/10", f"Sessions: {result['sessions']}", f"Provider requests: {result['provider_requests']}", f"Analysis complete: {result['analysis_complete']}/{result['analysis_snapshots']}", f"Unknown transformed: {result['unknown_transformed']}", "", "## Shadow accounting", "", f"Jobs admitted/processed/dropped: **{experiment_row.get('shadow_jobs_admitted', 0)} / {experiment_row.get('shadow_jobs_processed', 0)} / {experiment_row.get('shadow_job_drops', 0)}**", f"Candidate evaluations attempted/completed/dropped: **{accounting['counter_attempted']} / {accounting['counter_completed']} / {accounting['counter_drops']}**", f"Persisted candidate rows: **{accounting['persisted_candidate_rows']}**", "", "| Candidate | Readability | Addressable | Reduction | Recovery | Determinism |", "|---|---|---:|---:|---:|---:|"]
        for candidate in result["compressors"]:
            def percent(value: float | None) -> str:
                return "—" if value is None else f"{value * 100:.2f}%"
            lines.append(f"| `{candidate['id']}` | {', '.join(candidate['provider_readability'])} | {percent(candidate['addressable_token_share'])} | {percent(candidate['byte_reduction_ratio'])} | {percent(candidate['recovery_success_rate'])} | {percent(candidate['determinism_success_rate'])} |")
        lines += ["", "No provider-token, cache, cost, or quality claim is made. Active A/B remains blocked until a human-readable candidate passes the materiality gate."]
        args.output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(args.output_md)
        return 0
    finally:
        if daemon_process is not None:
            run([str(cli), "daemon", "stop"], cwd=repo, env=env, timeout=30)
            if daemon_process.poll() is None:
                daemon_process.terminate()
                try:
                    daemon_process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    daemon_process.kill()
                    daemon_process.wait(timeout=10)
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
