#!/usr/bin/env python3
"""Attribute repeated developer-text blocks by controlled Codex source arms."""

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


ARM_NAMES = (
    "user_config",
    "ignore_user_config",
    "ignore_user_config_workspace_marker",
    "ignore_user_config_developer_marker",
)
EXPERIMENT_ID = "instruction-source-attribution-016"
RIPGREP_REPOSITORY = "BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--sessions", type=int, default=3, choices=range(1, 6))
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument(
        "--arm-root",
        type=Path,
        help="offline directory containing <arm>.sqlite3 and <arm>.json evidence",
    )
    parser.add_argument("--codex-version")
    return parser.parse_args()


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


def _usage(connection: sqlite3.Connection) -> dict[str, dict[str, int | float | str | None]]:
    row = connection.execute(
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
    rows = int(row[0] or 0)

    def complete(count_index: int, value_index: int) -> int | None:
        if rows == 0 or int(row[count_index] or 0) != rows:
            return None
        return int(row[value_index] or 0)

    input_tokens = complete(1, 2)
    cached_tokens = complete(3, 4)
    uncached_tokens = complete(5, 6)
    return {
        "input_tokens": metric(input_tokens, "provider_reported"),
        "cached_input_tokens": metric(cached_tokens, "provider_reported"),
        "uncached_input_tokens": metric(uncached_tokens, "provider_reported"),
        "cache_ratio": metric(
            cached_tokens / input_tokens
            if cached_tokens is not None and input_tokens not in (None, 0)
            else None,
            "provider_reported",
        ),
        "output_tokens": metric(complete(7, 8), "provider_reported"),
        "reasoning_tokens": metric(complete(9, 10), "provider_reported"),
    }


def _analyze_arm(database: Path) -> tuple[dict[str, Any], dict[bytes, int]]:
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
        complete = int(complete or 0)
        requests = int(requests or 0)
        coverage_complete = requests > 0 and requests == snapshots == complete
        blocks, estimated, identified, total_tokens = connection.execute(
            """WITH latest AS (
                   SELECT cs.snapshot_id FROM context_snapshots cs
                   WHERE cs.analysis_version = (
                       SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2
                       WHERE cs2.provider_request_id = cs.provider_request_id
                   )
               )
               SELECT COUNT(*), COUNT(cbo.estimated_tokens), COUNT(cbo.exact_fingerprint),
                      SUM(cbo.estimated_tokens)
               FROM latest l
               JOIN context_block_occurrences cbo ON cbo.snapshot_id = l.snapshot_id
               WHERE cbo.kind = 'text' AND cbo.role = 'developer'
                 AND cbo.origin = 'human_authored'"""
        ).fetchone()
        blocks = int(blocks or 0)
        estimated = int(estimated or 0)
        identified = int(identified or 0)
        evidence_complete = coverage_complete and blocks == estimated == identified
        stable: dict[bytes, int] = {}
        if evidence_complete:
            rows = connection.execute(
                """WITH latest AS (
                       SELECT cs.snapshot_id, cs.provider_request_id
                       FROM context_snapshots cs
                       WHERE cs.analysis_version = (
                           SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2
                           WHERE cs2.provider_request_id = cs.provider_request_id
                       )
                   ), per_request AS (
                       SELECT l.provider_request_id, cbo.exact_fingerprint,
                              COUNT(*) AS occurrences,
                              MIN(cbo.estimated_tokens) AS min_tokens,
                              MAX(cbo.estimated_tokens) AS max_tokens
                       FROM latest l
                       JOIN context_block_occurrences cbo ON cbo.snapshot_id = l.snapshot_id
                       WHERE cbo.kind = 'text' AND cbo.role = 'developer'
                         AND cbo.origin = 'human_authored'
                       GROUP BY l.provider_request_id, cbo.exact_fingerprint
                   )
                   SELECT exact_fingerprint, COUNT(*), MIN(occurrences), MAX(occurrences),
                          MIN(min_tokens), MAX(max_tokens)
                   FROM per_request GROUP BY exact_fingerprint"""
            ).fetchall()
            for fingerprint, request_count, min_occurrences, max_occurrences, min_tokens, max_tokens in rows:
                if (
                    int(request_count) == requests
                    and int(min_occurrences) == int(max_occurrences)
                    and int(min_tokens) == int(max_tokens)
                ):
                    stable[bytes(fingerprint)] = int(min_occurrences) * int(min_tokens)
        usage = _usage(connection)
    stable_tokens = sum(stable.values()) if evidence_complete else None
    return (
        {
            "requests_observed": requests,
            "complete_observations": complete,
            "measurement_complete": evidence_complete,
            "developer_text_blocks": blocks if coverage_complete else None,
            "developer_estimated_tokens_total": metric(
                int(total_tokens or 0) if evidence_complete else None,
                "locally_estimated",
            ),
            "stable_block_classes": len(stable) if evidence_complete else None,
            "stable_tokens_per_request": stable_tokens,
            "provider_usage": usage,
        },
        stable,
    )


def attribute_databases(databases: dict[str, Path]) -> dict[str, Any]:
    if set(databases) != set(ARM_NAMES):
        raise ValueError("attribution requires the exact controlled arm set")
    arms: dict[str, dict[str, Any]] = {}
    stable: dict[str, dict[bytes, int]] = {}
    for name in ARM_NAMES:
        arms[name], stable[name] = _analyze_arm(databases[name])

    all_complete = all(arms[name]["measurement_complete"] for name in ARM_NAMES)
    common = set(stable[ARM_NAMES[0]])
    for name in ARM_NAMES[1:]:
        common &= set(stable[name])
    weight_consistent = all(
        len({stable[name][fingerprint] for name in ARM_NAMES}) == 1
        for fingerprint in common
    )
    user_keys = set(stable["user_config"])
    ignored_keys = set(stable["ignore_user_config"])
    workspace_keys = set(stable["ignore_user_config_workspace_marker"])
    developer_keys = set(stable["ignore_user_config_developer_marker"])

    def tokens(name: str, identities: set[bytes]) -> int | None:
        if not all_complete or not weight_consistent:
            return None
        return sum(stable[name][identity] for identity in identities)

    workspace_only = workspace_keys - ignored_keys
    workspace_removed = ignored_keys - workspace_keys
    developer_only = developer_keys - ignored_keys
    developer_removed = ignored_keys - developer_keys
    return {
        "mode": "shadow",
        "provider_effect_active": False,
        "arms": arms,
        "attribution": {
            "common_all_arms_tokens_per_request": tokens("user_config", common),
            "user_config_only_tokens_per_request": tokens(
                "user_config", user_keys - ignored_keys
            ),
            "ignored_config_only_tokens_per_request": tokens(
                "ignore_user_config", ignored_keys - user_keys
            ),
            "workspace_marker_only_tokens_per_request": tokens(
                "ignore_user_config_workspace_marker", workspace_only
            ),
            "workspace_marker_removed_tokens_per_request": tokens(
                "ignore_user_config", workspace_removed
            ),
            "developer_marker_only_tokens_per_request": tokens(
                "ignore_user_config_developer_marker", developer_only
            ),
            "developer_marker_removed_tokens_per_request": tokens(
                "ignore_user_config", developer_removed
            ),
        },
        "calibration": {
            "all_arms_complete": all_complete,
            "stable_weight_consistent": weight_consistent,
            "workspace_marker_delta_observed": bool(workspace_only or workspace_removed)
            if all_complete and weight_consistent
            else False,
            "developer_marker_delta_observed": bool(developer_only or developer_removed)
            if all_complete and weight_consistent
            else False,
        },
    }


def build_report(
    executions: dict[str, dict[str, Any]],
    attribution: dict[str, Any],
    sessions_requested: int,
    codex_version_value: str,
) -> dict[str, Any]:
    if set(executions) != set(ARM_NAMES):
        raise ValueError("execution evidence requires the exact controlled arm set")
    execution_summary: dict[str, dict[str, int | bool]] = {}
    executions_complete = True
    for name in ARM_NAMES:
        execution = executions[name].get("execution", {})
        summary = {
            "sessions_requested": sessions_requested,
            "sessions_completed": int(execution.get("sessions_completed", 0)),
            "sessions_return_code_zero": int(execution.get("sessions_return_code_zero", 0)),
            "sessions_return_code_nonzero": int(
                execution.get("sessions_return_code_nonzero", 0)
            ),
            "sessions_timed_out": int(execution.get("sessions_timed_out", 0)),
        }
        expected_user_config = name == "user_config"
        expected_workspace_profile = (
            "public_marker_v1"
            if name == "ignore_user_config_workspace_marker"
            else "none"
        )
        expected_developer_profile = (
            "public_marker_v1"
            if name == "ignore_user_config_developer_marker"
            else "none"
        )
        configuration_contract_valid = (
            executions[name].get("model") == MODEL
            and executions[name].get("repository_pin")
            == {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA}
            and executions[name].get("forwarding_mutations") == 0
            and executions[name].get("active_compression") == "off"
            and executions[name].get("shadow_compression") is False
            and executions[name].get("codex_user_config_loaded") is expected_user_config
            and executions[name].get("workspace_instruction_profile")
            == expected_workspace_profile
            and executions[name].get("developer_instruction_profile")
            == expected_developer_profile
        )
        summary["configuration_contract_valid"] = configuration_contract_valid
        execution_summary[name] = summary
        executions_complete &= (
            configuration_contract_valid
            and summary["sessions_completed"] == sessions_requested
            and summary["sessions_return_code_zero"] == sessions_requested
            and summary["sessions_timed_out"] == 0
        )
    evidence_complete = (
        executions_complete and attribution["calibration"]["all_arms_complete"]
    )
    calibration_passed = (
        evidence_complete
        and attribution["calibration"]["stable_weight_consistent"]
        and attribution["calibration"]["developer_marker_delta_observed"]
    )
    if calibration_passed:
        decision = "attribution_calibrated_policy_still_blocked"
    elif evidence_complete:
        decision = "attribution_calibration_failed"
    else:
        decision = "insufficient_attribution_evidence"
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "6.3",
        "status": "completed" if executions_complete else "completed_with_execution_gaps",
        "mode": "shadow",
        "workspace_class": "public_controlled",
        "repository_pin": {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA},
        "model": MODEL,
        "codex_version": codex_version_value,
        "sessions_per_arm": sessions_requested,
        "execution": execution_summary,
        "source_attribution": attribution,
        "gate": {
            "evidence_complete": evidence_complete,
            "calibration_passed": calibration_passed,
            "source_attribution_active": False,
            "instruction_policy_active": False,
            "provider_effect_active": False,
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
            "configuration_contents_in_report": False,
            "authentication_material_copied": False,
        },
        "limitations": [
            "Ignoring user config does not remove Codex built-ins or prove that every extension source is absent.",
            "The workspace marker is a calibration intervention, not a removal candidate.",
            "Stable exact-block attribution cannot decompose multiple sources concatenated into one text leaf.",
            "Provider usage is adjacent observational evidence and is not attributed savings.",
            "No ordinary Tracepress session instruction or provider request was modified.",
        ],
    }


def display(value: object) -> str:
    return "—" if value is None else str(value)


def markdown(report: dict[str, Any]) -> str:
    source = report["source_attribution"]
    attributed = source["attribution"]
    lines = [
        "# TRACEPRESS_INSTRUCTION_SOURCE_ATTRIBUTION_016",
        "",
        "Phase 6.3 controlled public instruction-source attribution. Shadow only; no ordinary provider request or instruction policy was modified.",
        "",
        f"Runtime: **{report['codex_version']}**. Model: **{report['model']}**. Sessions per arm: **{report['sessions_per_arm']}**.",
        "",
        f"Decision: **{report['gate']['decision']}**.",
        "",
        "## Arms",
        "",
        "| Arm | Requests | Complete | Stable tokens/request | Input | Cached | Uncached |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for name in ARM_NAMES:
        arm = source["arms"][name]
        usage = arm["provider_usage"]
        lines.append(
            f"| `{name}` | {arm['requests_observed']} | {arm['complete_observations']} | "
            f"{display(arm['stable_tokens_per_request'])} | "
            f"{display(usage['input_tokens']['value'])} | "
            f"{display(usage['cached_input_tokens']['value'])} | "
            f"{display(usage['uncached_input_tokens']['value'])} |"
        )
    lines.extend(
        [
            "",
            "## Structural attribution",
            "",
            "| Metric | Estimated tokens/request |",
            "|---|---:|",
            f"| Stable blocks common to all arms | {display(attributed['common_all_arms_tokens_per_request'])} |",
            f"| User-config-only stable blocks | {display(attributed['user_config_only_tokens_per_request'])} |",
            f"| Ignored-config-only stable blocks | {display(attributed['ignored_config_only_tokens_per_request'])} |",
            f"| Workspace-marker-only stable blocks | {display(attributed['workspace_marker_only_tokens_per_request'])} |",
            f"| Stable blocks removed/replaced by workspace marker | {display(attributed['workspace_marker_removed_tokens_per_request'])} |",
            f"| Developer-marker-only stable blocks | {display(attributed['developer_marker_only_tokens_per_request'])} |",
            f"| Stable blocks removed/replaced by developer marker | {display(attributed['developer_marker_removed_tokens_per_request'])} |",
            "",
            "Provider usage is observed separately and is not attributed savings.",
            "",
            "Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, fingerprint, semantic path, configuration content, authentication material, session id, or request id is in this report.",
            "",
        ]
    )
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


def load_arm_evidence(root: Path) -> tuple[dict[str, Path], dict[str, dict[str, Any]]]:
    databases = {name: root / f"{name}.sqlite3" for name in ARM_NAMES}
    executions = {
        name: json.loads((root / f"{name}.json").read_text(encoding="utf-8"))
        for name in ARM_NAMES
    }
    return databases, executions


def run_live_arms(
    repo_root: Path, root: Path, sessions: int, timeout: int
) -> tuple[dict[str, Path], dict[str, dict[str, Any]]]:
    workload = repo_root / "scripts/characterize_public_provider_native_search_001.py"
    databases: dict[str, Path] = {}
    executions: dict[str, dict[str, Any]] = {}
    environment = os.environ.copy()
    for name in ARM_NAMES:
        database = root / f"{name}.sqlite3"
        output_json = root / f"{name}.json"
        output_md = root / f"{name}.md"
        command = [
            sys.executable,
            str(workload),
            "--repo-root",
            str(repo_root),
            "--sessions",
            str(sessions),
            "--timeout",
            str(timeout),
            "--metadata-database-output",
            str(database),
            "--output-json",
            str(output_json),
            "--output-md",
            str(output_md),
        ]
        if name != "user_config":
            command.append("--ignore-user-config")
        if name == "ignore_user_config_workspace_marker":
            command.append("--workspace-instruction-marker-v1")
        if name == "ignore_user_config_developer_marker":
            command.append("--controlled-developer-instructions-v1")
        subprocess.run(
            command,
            cwd=repo_root,
            env=environment,
            check=True,
            capture_output=True,
            text=True,
            timeout=sessions * timeout + 180,
        )
        databases[name] = database
        executions[name] = json.loads(output_json.read_text(encoding="utf-8"))
    return databases, executions


def main() -> int:
    options = parse_args()
    repo_root = options.repo_root.resolve()
    version = options.codex_version or discover_codex_version()
    if options.arm_root is not None:
        databases, executions = load_arm_evidence(options.arm_root)
        attribution = attribute_databases(databases)
        report = build_report(executions, attribution, options.sessions, version)
    else:
        with tempfile.TemporaryDirectory(
            prefix="tracepress-instruction-attribution-016-", dir="/tmp"
        ) as raw:
            databases, executions = run_live_arms(
                repo_root, Path(raw), options.sessions, options.timeout
            )
            attribution = attribute_databases(databases)
            report = build_report(executions, attribution, options.sessions, version)
    write_report(report, options.output_json, options.output_md)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
