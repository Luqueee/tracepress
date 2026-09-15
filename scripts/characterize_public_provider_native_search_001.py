#!/usr/bin/env python3
"""Characterize provider-native Search ToolResult metadata on one public task.

This runs exactly one read-only Codex session against the pinned public ripgrep
checkout. Prompt, command, source, output, paths, and fingerprints remain
transient. The report is built from an explicit aggregate allowlist only.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
from typing import Any


EXPERIMENT_ID = "public-provider-native-search-shape-001"
SHADOW_EXPERIMENT_ID = "public-provider-native-search-envelope-shadow-001"
RIPGREP_URL = "https://github.com/BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument(
        "--shadow-compression",
        action="store_true",
        help="evaluate bounded shadow candidates without mutating forwarding",
    )
    return parser.parse_args()


def scalar(connection: sqlite3.Connection, query: str) -> int:
    return int(connection.execute(query).fetchone()[0] or 0)


def characterize(database: Path, shadow_experiment_id: str | None) -> dict[str, Any]:
    uri = f"file:{database}?mode=ro"
    with sqlite3.connect(uri, uri=True) as connection:
        connection.execute("PRAGMA query_only=ON")
        rows = connection.execute(
            """SELECT COALESCE(origin, 'unavailable'), COALESCE(kind, 'unavailable'),
                      COALESCE(role, 'unavailable'), COALESCE(detected_kind, 'unavailable'),
                      COUNT(*), SUM(raw_bytes), SUM(estimated_tokens),
                      MIN(raw_bytes), MAX(raw_bytes), SUM(CASE WHEN tool_name IS NOT NULL THEN 1 ELSE 0 END)
               FROM context_block_occurrences
               GROUP BY origin, kind, role, detected_kind
               ORDER BY SUM(COALESCE(estimated_tokens, 0)) DESC, COUNT(*) DESC"""
        ).fetchall()
        groups = [
            {
                "origin": origin,
                "block_kind": kind,
                "role": role,
                "detected_content_kind": detected_kind,
                "block_count": int(block_count),
                "raw_bytes": int(raw_bytes or 0),
                "estimated_tokens": None if estimated_tokens is None else int(estimated_tokens),
                "raw_bytes_min": int(raw_bytes_min or 0),
                "raw_bytes_max": int(raw_bytes_max or 0),
                "tool_identity_present_count": int(tool_identity_present or 0),
            }
            for (
                origin,
                kind,
                role,
                detected_kind,
                block_count,
                raw_bytes,
                estimated_tokens,
                raw_bytes_min,
                raw_bytes_max,
                tool_identity_present,
            ) in rows
        ]
        candidate_target = next(
            (
                group
                for group in groups
                if group["origin"] == "tool_generated"
                and group["block_kind"] == "tool_result"
                and group["detected_content_kind"] in {"plain_text", "search_results", "json"}
            ),
            None,
        )
        shadow_candidates: list[dict[str, Any]] = []
        if shadow_experiment_id is not None:
            candidate_rows = connection.execute(
                """SELECT c.compressor_id, c.status, COUNT(*),
                          SUM(m.input_bytes), SUM(m.output_bytes), SUM(m.bytes_delta),
                          SUM(m.recovery_verified), SUM(m.deterministic)
                   FROM compression_candidates c
                   JOIN compression_candidate_metrics m ON m.candidate_id = c.candidate_id
                   WHERE c.experiment_id = ?
                   GROUP BY c.compressor_id, c.status
                   ORDER BY c.compressor_id, c.status""",
                (shadow_experiment_id,),
            ).fetchall()
            shadow_candidates = [
                {
                    "candidate": candidate,
                    "status": status,
                    "evaluations": int(evaluations),
                    "input_bytes": int(input_bytes or 0),
                    "output_bytes": None if output_bytes is None else int(output_bytes),
                    "bytes_delta": None if bytes_delta is None else int(bytes_delta),
                    "recovery_verified": int(recovery_verified or 0),
                    "deterministic": int(deterministic or 0),
                }
                for (
                    candidate,
                    status,
                    evaluations,
                    input_bytes,
                    output_bytes,
                    bytes_delta,
                    recovery_verified,
                    deterministic,
                ) in candidate_rows
            ]
        return {
            "provider_requests": scalar(connection, "SELECT COUNT(*) FROM provider_requests"),
            "provider_errors": scalar(
                connection,
                "SELECT COUNT(*) FROM provider_attempts WHERE status <> 'completed' OR error_code IS NOT NULL OR transport_error IS NOT NULL",
            ),
            "context_snapshots": scalar(connection, "SELECT COUNT(*) FROM context_snapshots"),
            "context_snapshots_complete": scalar(
                connection, "SELECT COUNT(*) FROM context_snapshots WHERE status = 'complete'"
            ),
            "context_blocks": scalar(connection, "SELECT COUNT(*) FROM context_block_occurrences"),
            "groups": groups,
            "search_projection_target_observed": candidate_target is not None,
            "search_projection_target_metadata": candidate_target,
            "shadow_candidates": shadow_candidates,
        }


def prompt() -> str:
    # Public execution input only. It is intentionally absent from every artifact.
    return (
        "Inspect this public repository. Use the shell exactly once to run a bounded ripgrep "
        "search for Result< in Rust files, then answer with only the aggregate number of matching "
        "lines and unique files. Do not modify files."
    )


def main() -> int:
    options = parse_args()
    repo_root = options.repo_root.resolve()
    cli = repo_root / "target/debug/tracepress"
    daemon = repo_root / "target/debug/tracepressd"
    if not cli.exists() or not daemon.exists():
        raise RuntimeError("build target/debug/tracepress and target/debug/tracepressd first")

    workload_root = Path(tempfile.mkdtemp(prefix="tracepress-public-search-workload-", dir="/tmp"))
    state_root = Path(tempfile.mkdtemp(prefix="tracepress-native-shape-", dir="/tmp"))
    public_repo = workload_root / "ripgrep"
    daemon_process: subprocess.Popen[str] | None = None
    daemon_log = None
    environment: dict[str, str] | None = None
    try:
        subprocess.run(
            ["git", "clone", "--filter=blob:none", RIPGREP_URL, str(public_repo)],
            check=True,
            capture_output=True,
            text=True,
            timeout=60,
        )
        subprocess.run(
            ["git", "checkout", "--detach", RIPGREP_SHA],
            cwd=public_repo,
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
        environment = os.environ.copy()
        environment.update(
            {
                "TRACEPRESS_HOME": str(state_root),
                "TRACEPRESS_CONTEXT_ANALYSIS": "shadow",
                "TRACEPRESS_SHADOW_COMPRESSION": "on" if options.shadow_compression else "off",
                "TRACEPRESS_ACTIVE_COMPRESSION": "off",
                "TRACEPRESS_MEASUREMENT_RUN_ID": EXPERIMENT_ID,
            }
        )
        if options.shadow_compression:
            environment["TRACEPRESS_SHADOW_EXPERIMENT_ID"] = SHADOW_EXPERIMENT_ID
        daemon_environment = dict(environment)
        daemon_environment.update(
            {
                "TRACEPRESS_DATABASE": str(state_root / "tracepress.sqlite3"),
                "TRACEPRESS_CONTROL_SOCKET": str(state_root / "tracepress.sock"),
                "TRACEPRESS_CONTROL_CREDENTIAL": str(state_root / "control.cred"),
                "TRACEPRESS_DAEMON_READY": str(state_root / "daemon.ready"),
            }
        )
        subprocess.run(
            [str(cli), "init"], cwd=public_repo, env=environment, check=True,
            capture_output=True, text=True, timeout=30,
        )
        daemon_log = (state_root / "daemon.log").open("w", encoding="utf-8")
        daemon_process = subprocess.Popen(
            [str(daemon)], cwd=repo_root, env=daemon_environment, stdout=daemon_log,
            stderr=subprocess.STDOUT, start_new_session=True, text=True,
        )
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if daemon_process.poll() is not None:
                raise RuntimeError("daemon exited before readiness")
            if (state_root / "daemon.ready").exists():
                status = subprocess.run(
                    [str(cli), "daemon", "status"], cwd=public_repo, env=environment,
                    capture_output=True, text=True, timeout=30,
                )
                if status.returncode == 0:
                    break
            time.sleep(0.1)
        else:
            raise RuntimeError("daemon did not become ready")
        try:
            execution = subprocess.run(
                [str(cli), "run", "codex", "exec", "-m", MODEL, "-s", "read-only",
                 "--skip-git-repo-check", prompt()],
                cwd=public_repo, env=environment, capture_output=True, text=True,
                timeout=options.timeout,
            )
            timed_out = False
        except subprocess.TimeoutExpired:
            execution = subprocess.CompletedProcess([], 124, "", "")
            timed_out = True
        # Context Analysis and shadow evaluation are independent. Do not interpret a partially
        # flushed candidate set merely because snapshots are complete.
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            with sqlite3.connect(f"file:{state_root / 'tracepress.sqlite3'}?mode=ro", uri=True) as connection:
                pending = scalar(connection, "SELECT COUNT(*) FROM context_snapshots WHERE status <> 'complete'")
                shadow_complete = not options.shadow_compression
                if options.shadow_compression:
                    row = connection.execute(
                        "SELECT status, completed_at FROM compression_experiments WHERE experiment_id = ?",
                        (SHADOW_EXPERIMENT_ID,),
                    ).fetchone()
                    shadow_complete = row is not None and row[0] == "completed" and row[1] is not None
            if pending == 0 and shadow_complete:
                break
            time.sleep(0.2)
        observed = characterize(
            state_root / "tracepress.sqlite3",
            SHADOW_EXPERIMENT_ID if options.shadow_compression else None,
        )
        report = {
            "experiment_id": SHADOW_EXPERIMENT_ID if options.shadow_compression else EXPERIMENT_ID,
            "status": "completed" if not timed_out else "completed_evaluator_unavailable",
            "phase": "4.5",
            "workspace_class": "public_controlled",
            "repository_pin": {"repository": "BurntSushi/ripgrep", "commit_sha": RIPGREP_SHA},
            "model": MODEL,
            "forwarding_mutations": 0,
            "active_compression": "off",
            "shadow_compression": options.shadow_compression,
            "execution": {"return_code": execution.returncode, "timed_out": timed_out},
            "observed": observed,
            "privacy": {
                "report_contract": "aggregate_allowlist_only",
                "raw_content_persisted": False,
                "paths_persisted": False,
                "commands_persisted": False,
                "responses_persisted": False,
            },
            "limitations": [
                "One bounded public session characterizes provider-native metadata only.",
                "No efficacy, quality, cache-causality, or provider-savings conclusion follows from this diagnostic.",
            ],
        }
        options.output_json.parent.mkdir(parents=True, exist_ok=True)
        options.output_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        summary = [
            "# TRACEPRESS_PUBLIC_PROVIDER_NATIVE_SEARCH_SHAPE_001",
            "",
            "One bounded public metadata-only diagnostic of the provider-native Search ToolResult shape.",
            "",
            f"Provider requests: **{observed['provider_requests']}**. Context snapshots complete: **{observed['context_snapshots_complete']}/{observed['context_snapshots']}**.",
            f"Search-projection target observed: **{str(observed['search_projection_target_observed']).lower()}**.",
            "",
            "| Origin | Block kind | Role | Detected content kind | Blocks | Raw bytes | Estimated tokens |",
            "|---|---|---|---|---:|---:|---:|",
        ]
        for group in observed["groups"]:
            tokens = group["estimated_tokens"] if group["estimated_tokens"] is not None else "—"
            summary.append(
                f"| `{group['origin']}` | `{group['block_kind']}` | `{group['role']}` | `{group['detected_content_kind']}` | {group['block_count']} | {group['raw_bytes']} | {tokens} |"
            )
        if options.shadow_compression:
            summary.extend([
                "",
                "| Candidate | Status | Evaluations | Input bytes | Output bytes | Byte reduction | Recovery verified | Deterministic |",
                "|---|---|---:|---:|---:|---:|---:|---:|",
            ])
            for candidate in observed["shadow_candidates"]:
                output = candidate["output_bytes"] if candidate["output_bytes"] is not None else "—"
                reduction = candidate["bytes_delta"] if candidate["bytes_delta"] is not None else "—"
                summary.append(
                    f"| `{candidate['candidate']}` | `{candidate['status']}` | {candidate['evaluations']} | "
                    f"{candidate['input_bytes']} | {output} | {reduction} | "
                    f"{candidate['recovery_verified']} | {candidate['deterministic']} |"
                )
        summary.extend([
            "",
            "Privacy: aggregate allowlist only; no prompt, command, path, source, ToolResult, response, or fingerprint is in this report.",
        ])
        options.output_md.write_text("\n".join(summary) + "\n", encoding="utf-8")
        print("\n".join(summary))
        return 0
    finally:
        if daemon_process is not None and environment is not None:
            subprocess.run([str(cli), "daemon", "stop"], cwd=public_repo, env=environment, capture_output=True, text=True, timeout=30)
            if daemon_process.poll() is None:
                daemon_process.terminate()
                try:
                    daemon_process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    daemon_process.kill()
                    daemon_process.wait(timeout=10)
        if daemon_log is not None:
            daemon_log.close()
        shutil.rmtree(workload_root, ignore_errors=True)
        shutil.rmtree(state_root, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
