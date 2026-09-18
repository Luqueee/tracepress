#!/usr/bin/env python3
"""Measure repeated developer instruction exposure from Tracepress metadata."""

from __future__ import annotations

import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
from typing import Any


EXPERIMENT_ID = "instruction-surface-shadow-pilot-015"
RIPGREP_REPOSITORY = "BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--sessions", type=int, default=10, choices=range(1, 11))
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument("--metadata-database", type=Path)
    parser.add_argument("--execution-json", type=Path)
    parser.add_argument("--codex-version")
    options = parser.parse_args()
    if (options.metadata_database is None) != (options.execution_json is None):
        parser.error("--metadata-database and --execution-json must be provided together")
    return options


def discover_codex_version() -> str:
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


def metric(value: int | float | None, source: str) -> dict[str, int | float | str | None]:
    return {"value": value, "source": source if value is not None else "unavailable"}


def ratio_basis_points(part: int | None, total: int | None) -> int | None:
    if part is None or total is None or total == 0:
        return None
    return min(10_000, round(part * 10_000 / total))


def characterize_database(database: Path) -> dict[str, Any]:
    """Return an aggregate-only view of the latest analyzed provider requests."""
    uri = f"{database.resolve().as_uri()}?mode=ro"
    with closing(sqlite3.connect(uri, uri=True)) as connection:
        connection.execute("PRAGMA query_only=ON")
        snapshots, complete, requests = connection.execute(
            """WITH latest AS (
                   SELECT cs.* FROM context_snapshots cs
                   WHERE cs.analysis_version = (
                       SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2
                       WHERE cs2.provider_request_id = cs.provider_request_id
                   )
               ), observed_sessions AS (
                   SELECT DISTINCT session_id FROM latest
               )
               SELECT (SELECT COUNT(*) FROM latest),
                      SUM(CASE WHEN status = 'complete'
                                     AND explicit_request_complete = 1
                                     AND correlation_status = 'correlated'
                               THEN 1 ELSE 0 END),
                      (SELECT COUNT(*) FROM provider_requests pr
                       JOIN operations o ON o.operation_id = pr.operation_id
                       JOIN observed_sessions os ON os.session_id = o.session_id)
               FROM latest"""
        ).fetchone()
        snapshots = int(snapshots or 0)
        requests = int(requests or 0)
        complete = int(complete or 0)
        observations_complete = requests > 0 and requests == snapshots == complete

        (
            developer_blocks,
            developer_bytes,
            estimated_blocks,
            developer_tokens,
            identified_blocks,
        ) = connection.execute(
            """WITH latest AS (
                   SELECT cs.snapshot_id FROM context_snapshots cs
                   WHERE cs.analysis_version = (
                       SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2
                       WHERE cs2.provider_request_id = cs.provider_request_id
                   )
               )
               SELECT COUNT(*), SUM(cbo.raw_bytes), COUNT(cbo.estimated_tokens),
                      SUM(cbo.estimated_tokens), COUNT(cbo.exact_fingerprint)
               FROM latest l
               JOIN context_block_occurrences cbo ON cbo.snapshot_id = l.snapshot_id
               WHERE cbo.kind = 'text' AND cbo.role = 'developer'
                 AND cbo.origin = 'human_authored'"""
        ).fetchone()
        developer_blocks = int(developer_blocks or 0)
        developer_bytes = int(developer_bytes or 0)
        estimated_blocks = int(estimated_blocks or 0)
        identified_blocks = int(identified_blocks or 0)
        developer_tokens = int(developer_tokens or 0)
        developer_metrics_complete = (
            observations_complete
            and estimated_blocks == developer_blocks
            and identified_blocks == developer_blocks
        )

        recurring_request, recurring_session, repeat_exposure = connection.execute(
            """WITH latest AS (
                   SELECT cs.snapshot_id, cs.provider_request_id, cs.session_id
                   FROM context_snapshots cs
                   WHERE cs.analysis_version = (
                       SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2
                       WHERE cs2.provider_request_id = cs.provider_request_id
                   )
               ), grouped AS (
                   SELECT cbo.exact_fingerprint,
                          COUNT(DISTINCT l.provider_request_id) AS request_count,
                          COUNT(DISTINCT l.session_id) AS session_count,
                          SUM(cbo.estimated_tokens) AS total_tokens,
                          MIN(cbo.estimated_tokens) AS one_copy_tokens
                   FROM latest l
                   JOIN context_block_occurrences cbo ON cbo.snapshot_id = l.snapshot_id
                   WHERE cbo.kind = 'text' AND cbo.role = 'developer'
                     AND cbo.origin = 'human_authored'
                   GROUP BY cbo.exact_fingerprint
               )
               SELECT SUM(CASE WHEN request_count >= 2 THEN total_tokens ELSE 0 END),
                      SUM(CASE WHEN session_count >= 2 THEN total_tokens ELSE 0 END),
                      SUM(CASE WHEN request_count >= 2
                               THEN total_tokens - one_copy_tokens ELSE 0 END)
               FROM grouped"""
        ).fetchone()
        recurring_request = int(recurring_request or 0)
        recurring_session = int(recurring_session or 0)
        repeat_exposure = int(repeat_exposure or 0)

        unknown_blocks, unknown_bytes = connection.execute(
            """WITH latest AS (
                   SELECT cs.snapshot_id FROM context_snapshots cs
                   WHERE cs.analysis_version = (
                       SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2
                       WHERE cs2.provider_request_id = cs.provider_request_id
                   )
               )
               SELECT COUNT(*), SUM(cbo.raw_bytes)
               FROM latest l
               JOIN context_block_occurrences cbo ON cbo.snapshot_id = l.snapshot_id
               WHERE cbo.kind = 'unknown' AND cbo.role = 'developer'"""
        ).fetchone()

        usage = connection.execute(
            """WITH latest AS (
                   SELECT cs.provider_request_id FROM context_snapshots cs
                   WHERE cs.analysis_version = (
                       SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2
                       WHERE cs2.provider_request_id = cs.provider_request_id
                   )
               ), final_attempt AS (
                   SELECT l.provider_request_id, pa.attempt_id
                   FROM latest l
                   LEFT JOIN provider_attempts pa ON pa.request_id = l.provider_request_id
                     AND pa.ordinal = (
                         SELECT MAX(pa2.ordinal) FROM provider_attempts pa2
                         WHERE pa2.request_id = l.provider_request_id
                     )
               )
               SELECT COUNT(*), COUNT(pu.input_total), SUM(pu.input_total),
                      COUNT(pu.input_cached), SUM(pu.input_cached),
                      COUNT(pu.input_uncached), SUM(pu.input_uncached),
                      COUNT(pu.output_total), SUM(pu.output_total),
                      COUNT(pu.output_reasoning), SUM(pu.output_reasoning)
               FROM final_attempt fa
               LEFT JOIN provider_usage pu ON pu.attempt_id = fa.attempt_id"""
        ).fetchone()

    usage_rows = int(usage[0] or 0)

    def complete_usage(count_index: int, value_index: int) -> int | None:
        if usage_rows == 0 or int(usage[count_index] or 0) != usage_rows:
            return None
        return int(usage[value_index] or 0)

    input_tokens = complete_usage(1, 2)
    cached_tokens = complete_usage(3, 4)
    uncached_tokens = complete_usage(5, 6)
    output_tokens = complete_usage(7, 8)
    reasoning_tokens = complete_usage(9, 10)
    local_value = lambda value: value if developer_metrics_complete else None
    return {
        "mode": "shadow",
        "provider_effect_active": False,
        "requests_observed": requests,
        "complete_observations": complete,
        "observation_coverage_basis_points": ratio_basis_points(complete, requests),
        "developer_text_blocks": developer_blocks if observations_complete else None,
        "developer_text_bytes": metric(
            developer_bytes if observations_complete else None, "locally_measured"
        ),
        "developer_text_estimated_tokens": metric(
            local_value(developer_tokens), "locally_estimated"
        ),
        "cross_request_recurring_tokens": metric(
            local_value(recurring_request), "locally_estimated"
        ),
        "cross_session_recurring_tokens": metric(
            local_value(recurring_session), "locally_estimated"
        ),
        "repeat_exposure_tokens": metric(local_value(repeat_exposure), "locally_estimated"),
        "repeat_exposure_share_basis_points": ratio_basis_points(
            local_value(repeat_exposure), local_value(developer_tokens)
        ),
        "unknown_developer_blocks": int(unknown_blocks or 0) if observations_complete else None,
        "unknown_developer_bytes": int(unknown_bytes or 0) if observations_complete else None,
        "provider_usage": {
            "input_tokens": metric(input_tokens, "provider_reported"),
            "cached_input_tokens": metric(cached_tokens, "provider_reported"),
            "uncached_input_tokens": metric(uncached_tokens, "provider_reported"),
            "cache_ratio": metric(
                cached_tokens / input_tokens
                if cached_tokens is not None and input_tokens not in (None, 0)
                else None,
                "provider_reported",
            ),
            "output_tokens": metric(output_tokens, "provider_reported"),
            "reasoning_tokens": metric(reasoning_tokens, "provider_reported"),
        },
    }


def build_report(
    public_execution: dict[str, Any],
    surface: dict[str, Any],
    sessions_requested: int,
    codex_version_value: str,
) -> dict[str, Any]:
    execution = public_execution.get("execution", {})
    completed = execution.get("sessions_completed") == sessions_requested
    successful = execution.get("sessions_return_code_zero") == sessions_requested
    measurement_complete = (
        completed
        and successful
        and surface["observation_coverage_basis_points"] == 10_000
        and surface["developer_text_estimated_tokens"]["value"] is not None
        and surface["provider_usage"]["input_tokens"]["value"] is not None
    )
    recurring_tokens = surface["cross_session_recurring_tokens"]["value"]
    developer_tokens = surface["developer_text_estimated_tokens"]["value"]
    if measurement_complete and isinstance(recurring_tokens, int) and recurring_tokens > 0:
        decision = "source_attribution_required_before_policy_design"
    elif measurement_complete and isinstance(developer_tokens, int) and developer_tokens > 0:
        decision = "cross_session_repetition_not_observed"
    elif measurement_complete:
        decision = "no_instruction_surface_observed"
    else:
        decision = "insufficient_shadow_evidence"
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "6.2",
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
        "instruction_surface": surface,
        "shadow_gate": {
            "measurement_complete": measurement_complete,
            "cross_session_repetition_observed": isinstance(recurring_tokens, int)
            and recurring_tokens > 0,
            "source_attribution_complete": False,
            "provider_effect_active": False,
            "instruction_policy_active": False,
            "decision": decision,
        },
        "privacy": {
            "report_contract": "aggregate_allowlist_only",
            "raw_content_persisted": False,
            "paths_persisted": False,
            "commands_persisted": False,
            "responses_persisted": False,
            "fingerprints_in_report": False,
            "semantic_paths_in_report": False,
            "session_or_request_ids_in_report": False,
        },
        "limitations": [
            "Developer text is a leaf-only lower bound; parent message bytes are excluded to avoid overlap.",
            "Unknown developer blocks are measured only by count and bytes, without estimating tokens.",
            "Exact fingerprints are used ephemerally for aggregation and never leave the local database query.",
            "Provider cache usage is adjacent observational evidence, not attributed savings.",
            "Shadow mode does not remove, rewrite, reorder, or select instructions.",
        ],
    }


def display(value: object) -> str:
    return "—" if value is None else str(value)


def markdown(report: dict[str, Any]) -> str:
    surface = report["instruction_surface"]
    usage = surface["provider_usage"]
    gate = report["shadow_gate"]
    lines = [
        "# TRACEPRESS_INSTRUCTION_SURFACE_SHADOW_PILOT_015",
        "",
        "Phase 6.2 controlled public instruction-surface characterization. Shadow only; no provider request or instruction was modified.",
        "",
        f"Runtime: **{report['codex_version']}**. Model: **{report['model']}**.",
        "",
        f"Sessions: **{report['execution']['sessions_completed']}/{report['execution']['sessions_requested']}**. Decision: **{gate['decision']}**.",
        "",
        "## Developer text lower bound",
        "",
        "| Metric | Value |",
        "|---|---:|",
        f"| Requests observed | {surface['requests_observed']} |",
        f"| Complete observations | {surface['complete_observations']} |",
        f"| Coverage (basis points) | {display(surface['observation_coverage_basis_points'])} |",
        f"| Developer text blocks | {display(surface['developer_text_blocks'])} |",
        f"| Developer text bytes | {display(surface['developer_text_bytes']['value'])} |",
        f"| Developer estimated tokens | {display(surface['developer_text_estimated_tokens']['value'])} |",
        f"| Cross-request recurring tokens | {display(surface['cross_request_recurring_tokens']['value'])} |",
        f"| Cross-session recurring tokens | {display(surface['cross_session_recurring_tokens']['value'])} |",
        f"| Repeat exposure tokens | {display(surface['repeat_exposure_tokens']['value'])} |",
        f"| Repeat exposure share (basis points) | {display(surface['repeat_exposure_share_basis_points'])} |",
        f"| Unknown developer blocks | {display(surface['unknown_developer_blocks'])} |",
        f"| Unknown developer bytes | {display(surface['unknown_developer_bytes'])} |",
        "",
        "## Provider usage (observed, not attributed)",
        "",
        "| Metric | Value |",
        "|---|---:|",
        f"| Input | {display(usage['input_tokens']['value'])} |",
        f"| Cached | {display(usage['cached_input_tokens']['value'])} |",
        f"| Uncached | {display(usage['uncached_input_tokens']['value'])} |",
        f"| Cache ratio | {display(usage['cache_ratio']['value'])} |",
        f"| Output | {display(usage['output_tokens']['value'])} |",
        f"| Reasoning | {display(usage['reasoning_tokens']['value'])} |",
        "",
        "Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, fingerprint, semantic path, session id, or request id is in this report.",
        "",
    ]
    return "\n".join(lines)


def write_report(report: dict[str, Any], output_json: Path, output_md: Path) -> None:
    output_json.parent.mkdir(parents=True, exist_ok=True)
    output_md.parent.mkdir(parents=True, exist_ok=True)
    output_json.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    rendered = markdown(report)
    output_md.write_text(rendered, encoding="utf-8")
    print(rendered, end="")


def main() -> int:
    options = parse_args()
    repo_root = options.repo_root.resolve()
    version = options.codex_version or discover_codex_version()
    if options.metadata_database is not None:
        public_execution = json.loads(options.execution_json.read_text(encoding="utf-8"))
        surface = characterize_database(options.metadata_database)
    else:
        cli = repo_root / "target/debug/tracepress"
        daemon = repo_root / "target/debug/tracepressd"
        workload = repo_root / "scripts/characterize_public_provider_native_search_001.py"
        if not cli.is_file() or not daemon.is_file():
            raise RuntimeError("build target/debug/tracepress and target/debug/tracepressd first")
        with tempfile.TemporaryDirectory(
            prefix="tracepress-instruction-surface-015-", dir="/tmp"
        ) as raw:
            state_root = Path(raw)
            public_json = state_root / "public-execution.json"
            public_md = state_root / "public-execution.md"
            database = state_root / "tracepress.sqlite3"
            environment = os.environ.copy()
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
                env=environment,
                check=True,
                capture_output=True,
                text=True,
                timeout=options.sessions * options.timeout + 180,
            )
            public_execution = json.loads(public_json.read_text(encoding="utf-8"))
            surface = characterize_database(database)
    report = build_report(public_execution, surface, options.sessions, version)
    write_report(report, options.output_json, options.output_md)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
