#!/usr/bin/env python3
"""Run the Phase 6.7 single-process cache-key equality Shadow cohort."""

from __future__ import annotations

import argparse
from collections import Counter
from contextlib import closing
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
from typing import Any


ARM_NAMES = (
    "hooks_implicit_enabled",
    "hooks_explicit_enabled",
    "hooks_explicit_disabled",
)
EXPERIMENT_ID = "cache-key-equality-020"
RIPGREP_REPOSITORY = "BurntSushi/ripgrep"
RIPGREP_URL = "https://github.com/BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"
EXPECTED_RUNS = 18
EXPECTED_REQUESTS_PER_RUN = 2


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument("--evidence-root", type=Path)
    parser.add_argument("--codex-version")
    return parser.parse_args()


def discover_codex_version(real_codex: Path | None = None) -> str:
    result = subprocess.run(
        [str(real_codex or "codex"), "--version"],
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
    )
    version = result.stdout.strip()
    if not version or len(version) > 128 or "\n" in version:
        raise ValueError("unexpected Codex version output")
    return version


def _provider_rows(database: Path) -> list[dict[str, int | None]]:
    uri = f"{database.resolve().as_uri()}?mode=ro"
    with closing(sqlite3.connect(uri, uri=True)) as connection:
        connection.execute("PRAGMA query_only=ON")
        rows = connection.execute(
            """SELECT pu.input_total, pu.input_cached, pu.input_uncached,
                      pu.output_total, pu.output_reasoning
               FROM provider_requests pr
               JOIN provider_attempts pa ON pa.request_id = pr.request_id
                 AND pa.ordinal = (
                     SELECT MAX(pa2.ordinal) FROM provider_attempts pa2
                     WHERE pa2.request_id = pr.request_id
                 )
               LEFT JOIN provider_usage pu ON pu.attempt_id = pa.attempt_id
               ORDER BY pr.rowid"""
        ).fetchall()
    return [
        {
            "input_tokens": row[0],
            "cached_input_tokens": row[1],
            "uncached_input_tokens": row[2],
            "output_tokens": row[3],
            "reasoning_tokens": row[4],
        }
        for row in rows
    ]


def analyze(cohort: dict[str, Any], database: Path) -> dict[str, Any]:
    runs = cohort.get("runs", [])
    equality = cohort.get("cache_key_equality", {})
    records = equality.get("records", [])
    provider_rows = _provider_rows(database)
    run_contract = (
        len(runs) == EXPECTED_RUNS
        and all(run.get("return_code") == 0 and run.get("timed_out") is False for run in runs)
    )
    record_contract = (
        len(records) >= EXPECTED_RUNS
        and len(provider_rows) == len(records)
        and all(record.get("arm") in ARM_NAMES for record in records)
    )
    statuses = Counter(str(record.get("cache_key_status")) for record in records)
    present_records = [record for record in records if record.get("cache_key_status") == "present"]
    classes = {record.get("equality_class") for record in present_records}
    classes.discard(None)

    grouped: dict[tuple[str, int], list[dict[str, Any]]] = {}
    for record in records:
        grouped.setdefault((str(record.get("arm")), int(record.get("round", -1))), []).append(record)
    every_run_observed = len(grouped) == EXPECTED_RUNS and all(group for group in grouped.values())
    within_run_same = every_run_observed and all(
        len({record.get("equality_class") for record in group}) == 1
        and all(record.get("cache_key_status") == "present" for record in group)
        for group in grouped.values()
    )

    first_classes: dict[tuple[str, int], int | None] = {}
    for key, group in grouped.items():
        first = min(group, key=lambda record: int(record.get("request_index", -1)))
        value = first.get("equality_class")
        first_classes[key] = int(value) if value is not None else None
    matched_round_equalities: dict[str, int] = {}
    for left_index, left in enumerate(ARM_NAMES):
        for right in ARM_NAMES[left_index + 1 :]:
            matched_round_equalities[f"{left}__{right}"] = sum(
                first_classes.get((left, round_index)) is not None
                and first_classes.get((left, round_index))
                == first_classes.get((right, round_index))
                for round_index in range(6)
            )

    arm_usage = {
        arm: {
            metric: sum(
                int(provider_rows[index][metric] or 0)
                for index, record in enumerate(records)
                if record.get("arm") == arm
            )
            for metric in (
                "input_tokens",
                "cached_input_tokens",
                "uncached_input_tokens",
                "output_tokens",
                "reasoning_tokens",
            )
        }
        for arm in ARM_NAMES
    }
    all_present = len(present_records) == len(records) and bool(records)
    unique_per_run = (
        within_run_same and len(classes) == EXPECTED_RUNS
        and all(value == 0 for value in matched_round_equalities.values())
    )
    request_count_distribution = dict(sorted(Counter(len(group) for group in grouped.values()).items()))
    if not run_contract or not record_contract or not every_run_observed:
        classification = "insufficient_equality_evidence"
    elif statuses.get("absent", 0) == len(records):
        classification = "cache_key_absent"
    elif all_present and len(classes) == 1:
        classification = "one_shared_cache_key_class"
    elif all_present and unique_per_run:
        classification = "session_scoped_cache_key_classes"
    elif all_present:
        classification = "mixed_cache_key_classes"
    else:
        classification = "incomplete_cache_key_presence"

    return {
        "execution_contract_complete": run_contract,
        "record_contract_complete": record_contract,
        "runs_observed": len(runs),
        "requests_observed": len(records),
        "provider_requests_observed": len(provider_rows),
        "cache_key_status_counts": dict(sorted(statuses.items())),
        "unique_equality_classes": len(classes),
        "requests_per_run_distribution": {
            str(count): runs for count, runs in request_count_distribution.items()
        },
        "within_run_class_stable": within_run_same,
        "matched_round_cross_arm_equalities": matched_round_equalities,
        "arm_provider_usage": arm_usage,
        "classification": classification,
        "raw_key_values_persisted": False,
        "reusable_key_hashes_persisted": False,
    }


def build_report(analysis: dict[str, Any], codex_version: str) -> dict[str, Any]:
    evidence_complete = (
        analysis["execution_contract_complete"] and analysis["record_contract_complete"]
    )
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "6.7",
        "status": "completed" if evidence_complete else "completed_with_evidence_gaps",
        "mode": "shadow",
        "workspace_class": "public_controlled",
        "repository_pin": {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA},
        "model": MODEL,
        "codex_version": codex_version,
        "cache_key_equality": analysis,
        "gate": {
            "evidence_complete": evidence_complete,
            "cache_key_values_observed_transiently": True,
            "cache_key_values_persisted": False,
            "reusable_key_hashes_persisted": False,
            "task_quality_evaluated": False,
            "instruction_policy_active": False,
            "provider_effect_active": False,
            "decision": analysis["classification"],
        },
        "privacy": {
            "report_contract": "aggregate_equality_allowlist_only",
            "raw_content_persisted": False,
            "paths_persisted": False,
            "commands_persisted": False,
            "responses_persisted": False,
            "fingerprints_in_report": False,
            "cache_key_values_persisted": False,
            "reusable_key_hashes_persisted": False,
            "authentication_material_copied": False,
        },
        "limitations": [
            "Equality classes are ordinal labels scoped to one process and have no value outside it.",
            "Cache-key equality does not prove the provider's routing or cache lookup algorithm.",
            "The public Search workload and one Codex version do not generalize to arbitrary tasks.",
            "No ordinary Tracepress instruction or provider request was modified.",
        ],
    }


def markdown(report: dict[str, Any]) -> str:
    analysis = report["cache_key_equality"]
    lines = [
        "# TRACEPRESS_CACHE_KEY_EQUALITY_020",
        "",
        "Phase 6.7 single-process ephemeral cache-key equality Shadow study.",
        "",
        f"Decision: **{report['gate']['decision']}**.",
        "",
        f"Runs/requests/provider requests: **{analysis['runs_observed']}/{analysis['requests_observed']}/{analysis['provider_requests_observed']}**.",
        f"Cache-key status counts: **{json.dumps(analysis['cache_key_status_counts'], sort_keys=True)}**.",
        f"Unique ephemeral equality classes: **{analysis['unique_equality_classes']}**.",
        f"Requests per run: **{json.dumps(analysis['requests_per_run_distribution'], sort_keys=True)}**.",
        f"Same class across all requests within every run: **{analysis['within_run_class_stable']}**.",
        "",
        "## Cross-arm equality",
        "",
        "| Matched arm pair | Equal first-request classes |",
        "|---|---:|",
    ]
    for pair, count in analysis["matched_round_cross_arm_equalities"].items():
        lines.append(f"| `{pair}` | {count}/6 |")
    lines.extend(
        [
            "",
            "## Provider usage",
            "",
            "| Arm | Input | Cached | Uncached | Output | Reasoning |",
            "|---|---:|---:|---:|---:|---:|",
        ]
    )
    for arm in ARM_NAMES:
        usage = analysis["arm_provider_usage"][arm]
        lines.append(
            f"| `{arm}` | {usage['input_tokens']} | {usage['cached_input_tokens']} | "
            f"{usage['uncached_input_tokens']} | {usage['output_tokens']} | "
            f"{usage['reasoning_tokens']} |"
        )
    lines.extend(
        [
            "",
            "Raw cache-key values existed only in process memory. The report contains ordinal equality classes only as aggregate counts; it contains no key value or reusable hash.",
            "",
        ]
    )
    return "\n".join(lines)


def write_report(report: dict[str, Any], output_json: Path, output_md: Path) -> None:
    output_json.parent.mkdir(parents=True, exist_ok=True)
    output_md.parent.mkdir(parents=True, exist_ok=True)
    output_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    rendered = markdown(report)
    output_md.write_text(rendered, encoding="utf-8")
    print(rendered, end="")


def _scalar(database: Path, query: str) -> int:
    with closing(sqlite3.connect(f"{database.resolve().as_uri()}?mode=ro", uri=True)) as connection:
        return int(connection.execute(query).fetchone()[0] or 0)


def collect_live(repo_root: Path, root: Path, timeout: int) -> tuple[dict[str, Any], Path, Path]:
    cli = repo_root / "target/debug/tracepress"
    daemon = repo_root / "target/debug/tracepressd"
    child = repo_root / "scripts/run_cache_key_equality_child_020.py"
    if not cli.exists() or not daemon.exists():
        raise RuntimeError("build target/debug/tracepress and target/debug/tracepressd first")
    real_codex_raw = shutil.which("codex")
    if real_codex_raw is None:
        raise RuntimeError("codex executable is unavailable")
    real_codex = Path(real_codex_raw).resolve()

    public_repo = root / "ripgrep"
    state_root = root / "state"
    shim_root = root / "shim"
    shim_root.mkdir()
    state_root.mkdir()
    shutil.copy2(child, shim_root / "codex")
    (shim_root / "codex").chmod(0o700)
    subprocess.run(
        ["git", "clone", "--filter=blob:none", RIPGREP_URL, str(public_repo)],
        check=True, capture_output=True, text=True, timeout=60,
    )
    subprocess.run(
        ["git", "checkout", "--detach", RIPGREP_SHA], cwd=public_repo,
        check=True, capture_output=True, text=True, timeout=30,
    )
    environment = os.environ.copy()
    environment.update(
        {
            "PATH": f"{shim_root}{os.pathsep}{environment.get('PATH', '')}",
            "TRACEPRESS_HOME": str(state_root),
            "TRACEPRESS_CONTEXT_ANALYSIS": "shadow",
            "TRACEPRESS_SHADOW_COMPRESSION": "off",
            "TRACEPRESS_ACTIVE_COMPRESSION": "off",
            "TRACEPRESS_MEASUREMENT_RUN_ID": EXPERIMENT_ID,
        }
    )
    daemon_environment = dict(environment)
    database = state_root / "tracepress.sqlite3"
    daemon_environment.update(
        {
            "TRACEPRESS_DATABASE": str(database),
            "TRACEPRESS_CONTROL_SOCKET": str(state_root / "tracepress.sock"),
            "TRACEPRESS_CONTROL_CREDENTIAL": str(state_root / "control.cred"),
            "TRACEPRESS_DAEMON_READY": str(state_root / "daemon.ready"),
        }
    )
    subprocess.run(
        [str(cli), "init"], cwd=public_repo, env=environment,
        check=True, capture_output=True, text=True, timeout=30,
    )
    daemon_log = (state_root / "daemon.log").open("w", encoding="utf-8")
    daemon_process = subprocess.Popen(
        [str(daemon)], cwd=repo_root, env=daemon_environment, stdout=daemon_log,
        stderr=subprocess.STDOUT, start_new_session=True, text=True,
    )
    cohort_output = root / "cohort.json"
    try:
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
        execution = subprocess.run(
            [
                str(cli), "run", "codex", "--", "--cohort-output", str(cohort_output),
                "--timeout", str(timeout), "--real-codex", str(real_codex),
            ],
            cwd=public_repo, env=environment, capture_output=True, text=True,
            timeout=EXPECTED_RUNS * timeout + 300,
        )
        if not cohort_output.exists():
            raise RuntimeError(
                f"cohort child failed before evidence publication (return code {execution.returncode})"
            )
        cohort = json.loads(cohort_output.read_text(encoding="utf-8"))
        deadline = time.monotonic() + 30
        expected_requests = EXPECTED_RUNS * EXPECTED_REQUESTS_PER_RUN
        while time.monotonic() < deadline:
            requests = _scalar(database, "SELECT COUNT(*) FROM provider_requests")
            pending = _scalar(database, "SELECT COUNT(*) FROM context_snapshots WHERE status <> 'complete'")
            if requests == expected_requests and pending == 0:
                break
            time.sleep(0.2)
        evidence_database = root / "evidence.sqlite3"
        with closing(sqlite3.connect(database)) as source:
            with closing(sqlite3.connect(evidence_database)) as destination:
                source.backup(destination)
        return cohort, evidence_database, real_codex
    finally:
        subprocess.run(
            [str(cli), "daemon", "stop"], cwd=public_repo, env=environment,
            capture_output=True, text=True, timeout=30,
        )
        if daemon_process.poll() is None:
            daemon_process.terminate()
            try:
                daemon_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                daemon_process.kill()
                daemon_process.wait(timeout=10)
        daemon_log.close()


def main() -> int:
    options = parse_args()
    repo_root = options.repo_root.resolve()
    if options.evidence_root is not None:
        cohort = json.loads(
            (options.evidence_root / "cohort.json").read_text(encoding="utf-8")
        )
        database = options.evidence_root / "evidence.sqlite3"
        version = options.codex_version or discover_codex_version()
        analysis = analyze(cohort, database)
    else:
        with tempfile.TemporaryDirectory(prefix="tracepress-cache-key-equality-020-", dir="/tmp") as raw:
            cohort, database, real_codex = collect_live(repo_root, Path(raw), options.timeout)
            version = options.codex_version or discover_codex_version(real_codex)
            analysis = analyze(cohort, database)
    report = build_report(analysis, version)
    write_report(report, options.output_json, options.output_md)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
