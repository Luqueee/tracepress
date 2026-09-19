#!/usr/bin/env python3
"""Attribute the Phase 6.5 cache split to effective hooks state or override presence."""

from __future__ import annotations

import argparse
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
from typing import Any


ARM_NAMES = (
    "hooks_implicit_enabled",
    "hooks_explicit_enabled",
    "hooks_explicit_disabled",
)
ARM_ENABLED: dict[str, tuple[str, ...]] = {
    "hooks_implicit_enabled": (),
    "hooks_explicit_enabled": ("hooks",),
    "hooks_explicit_disabled": (),
}
ARM_DISABLED: dict[str, tuple[str, ...]] = {
    "hooks_implicit_enabled": (),
    "hooks_explicit_enabled": (),
    "hooks_explicit_disabled": ("hooks",),
}
EXPERIMENT_ID = "cache-key-attribution-019"
RIPGREP_REPOSITORY = "BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"
EQUIVALENT_UNCACHED_THRESHOLD = 0.10
SEPARATION_UNCACHED_THRESHOLD = 0.20
REQUEST_METADATA_FIELDS = (
    "request_bytes",
    "model",
    "stream",
    "background",
    "store",
    "reasoning_effort",
    "text_verbosity",
    "truncation",
    "previous_response_id_present",
    "input_item_count",
    "tool_count",
    "text_input_block_count",
    "image_input_block_count",
    "file_input_block_count",
    "observation_status",
    "uses_previous_response",
    "uses_conversation_state",
    "uses_item_references",
    "uses_prompt_reference",
    "uses_external_files",
    "uses_external_images",
    "contains_opaque_items",
    "logical_context_status",
    "duplicate_key_detected",
)
BLOCK_MANIFEST_FIELDS = (
    "ordinal",
    "kind",
    "role",
    "origin",
    "semantic_path",
    "raw_bytes",
    "exact_fingerprint",
    "estimated_tokens",
    "detected_kind",
    "tool_name",
)


def _load(name: str, filename: str) -> Any:
    path = Path(__file__).with_name(filename)
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load {filename}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PHASE_6_4 = _load("user_config_layer_decomposition_017", "run_user_config_layer_decomposition_017.py")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    parser.add_argument(
        "--arm-root",
        type=Path,
        help="offline directory containing <arm>-<round>.sqlite3 and .json evidence",
    )
    parser.add_argument("--codex-version")
    return parser.parse_args()


def discover_codex_version() -> str:
    result = subprocess.run(
        ["codex", "--version"], check=True, capture_output=True, text=True, timeout=10
    )
    version = result.stdout.strip()
    if not version or len(version) > 128 or "\n" in version:
        raise ValueError("unexpected Codex version output")
    return version


def balanced_schedule() -> tuple[tuple[str, ...], ...]:
    implicit, enabled, disabled = ARM_NAMES
    return (
        (implicit, enabled, disabled),
        (enabled, disabled, implicit),
        (disabled, implicit, enabled),
        (implicit, disabled, enabled),
        (disabled, enabled, implicit),
        (enabled, implicit, disabled),
    )


def _distance(left: int | float | None, right: int | float | None) -> float | None:
    if left is None or right is None:
        return None
    midpoint = (float(left) + float(right)) / 2
    return abs(float(left) - float(right)) / max(midpoint, 1.0)


def _initial_request_manifest(database: Path) -> dict[str, Any]:
    """Return content-free metadata for the first request in one controlled session."""
    uri = f"{database.resolve().as_uri()}?mode=ro"
    with closing(sqlite3.connect(uri, uri=True)) as connection:
        connection.execute("PRAGMA query_only=ON")
        row = connection.execute(
            """SELECT pr.request_id, pr.request_bytes, pr.model, pr.stream, pr.background,
                      pr.store, pr.reasoning_effort, pr.text_verbosity, pr.truncation,
                      pr.previous_response_id_present, pr.input_item_count, pr.tool_count,
                      pr.text_input_block_count, pr.image_input_block_count,
                      pr.file_input_block_count, pr.observation_status
               FROM provider_requests pr
               JOIN operations o ON o.operation_id = pr.operation_id
               ORDER BY o.started_at, pr.rowid LIMIT 1"""
        ).fetchone()
        if row is None:
            return {"complete": False, "request_metadata": None, "block_manifest": None}
        request_id = str(row[0])
        snapshot = connection.execute(
            """SELECT snapshot_id, status, explicit_request_complete, correlation_status,
                      uses_previous_response, uses_conversation_state, uses_item_references,
                      uses_prompt_reference, uses_external_files, uses_external_images,
                      contains_opaque_items, logical_context_status, duplicate_key_detected
               FROM context_snapshots WHERE provider_request_id = ?
               ORDER BY analysis_version DESC LIMIT 1""",
            (request_id,),
        ).fetchone()
        if snapshot is None:
            return {"complete": False, "request_metadata": None, "block_manifest": None}
        blocks = connection.execute(
            """SELECT ordinal, kind, role, origin, semantic_path, raw_bytes,
                      exact_fingerprint, estimated_tokens, detected_kind, tool_name
               FROM context_block_occurrences WHERE snapshot_id = ? ORDER BY ordinal""",
            (snapshot[0],),
        ).fetchall()
        complete = (
            row[15] == "complete"
            and snapshot[1] == "complete"
            and snapshot[2] == 1
            and snapshot[3] == "correlated"
            and all(block[6] is not None for block in blocks)
        )
        request_metadata = tuple(row[1:]) + tuple(snapshot[4:])
        block_manifest = tuple(
            tuple(block[:6])
            + (bytes(block[6]) if block[6] is not None else None,)
            + tuple(block[7:])
            for block in blocks
        )
        return {
            "complete": complete,
            "request_metadata": request_metadata,
            "block_manifest": block_manifest,
            "request_bytes": row[1],
            "block_count": len(blocks),
            "estimated_tokens": (
                sum(int(block[7]) for block in blocks)
                if blocks and all(block[7] is not None for block in blocks)
                else None
            ),
        }


def _usage(arm: dict[str, Any], key: str) -> int | None:
    value = arm["provider_usage"][key]["value"]
    return None if value is None else int(value)


def _mismatch_summary(
    manifests: dict[str, list[dict[str, Any]]], rounds: int
) -> dict[str, Any]:
    metadata_counts = {field: 0 for field in REQUEST_METADATA_FIELDS}
    block_field_counts = {field: 0 for field in BLOCK_MANIFEST_FIELDS}
    divergent_classes: dict[tuple[int, str, str, str, str | None], dict[str, Any]] = {}
    for round_index in range(rounds):
        for field_index, field in enumerate(REQUEST_METADATA_FIELDS):
            values = {
                manifests[name][round_index]["request_metadata"][field_index]
                for name in ARM_NAMES
            }
            metadata_counts[field] += len(values) > 1
        arm_blocks = [manifests[name][round_index]["block_manifest"] for name in ARM_NAMES]
        if len({len(blocks) for blocks in arm_blocks}) != 1:
            continue
        for block_index in range(len(arm_blocks[0])):
            rows = [blocks[block_index] for blocks in arm_blocks]
            mismatched_fields = []
            for field_index, field in enumerate(BLOCK_MANIFEST_FIELDS):
                if len({row[field_index] for row in rows}) > 1:
                    block_field_counts[field] += 1
                    mismatched_fields.append(field)
            if not mismatched_fields:
                continue
            exemplar = rows[0]
            key = (
                int(exemplar[0]),
                str(exemplar[1]),
                str(exemplar[2]),
                str(exemplar[3]),
                None if exemplar[4] is None else str(exemplar[4]),
            )
            entry = divergent_classes.setdefault(
                key,
                {
                    "ordinal": key[0],
                    "kind": key[1],
                    "role": key[2],
                    "origin": key[3],
                    "semantic_path": key[4],
                    "rounds_divergent": 0,
                    "mismatched_fields": set(),
                },
            )
            entry["rounds_divergent"] += 1
            entry["mismatched_fields"].update(mismatched_fields)
    classes = []
    for entry in divergent_classes.values():
        entry["mismatched_fields"] = sorted(entry["mismatched_fields"])
        classes.append(entry)
    classes.sort(key=lambda entry: (entry["ordinal"], entry["semantic_path"] or ""))
    return {
        "request_metadata_mismatch_rounds": {
            field: count for field, count in metadata_counts.items() if count
        },
        "block_field_mismatch_cells": {
            field: count for field, count in block_field_counts.items() if count
        },
        "divergent_block_classes": classes,
    }


def analyze(databases: dict[str, list[Path]]) -> dict[str, Any]:
    schedule = balanced_schedule()
    if set(databases) != set(ARM_NAMES):
        raise ValueError("attribution requires the exact controlled arm set")
    if any(len(databases[name]) != len(schedule) for name in ARM_NAMES):
        raise ValueError("every arm requires one database per crossover round")

    arms: dict[str, dict[str, Any]] = {}
    developer_maps: dict[str, dict[bytes, int]] = {}
    manifests: dict[str, list[dict[str, Any]]] = {}
    for name in ARM_NAMES:
        arms[name], developer_maps[name] = PHASE_6_4._aggregate_arm(databases[name])
        manifests[name] = [_initial_request_manifest(path) for path in databases[name]]

    all_complete = all(
        arms[name]["measurement_complete"]
        and all(manifest["complete"] for manifest in manifests[name])
        for name in ARM_NAMES
    )
    developer_maps_match = len({repr(developer_maps[name]) for name in ARM_NAMES}) == 1
    request_metadata_match = all(
        len({manifests[name][round_index]["request_metadata"] for name in ARM_NAMES}) == 1
        for round_index in range(len(schedule))
    )
    initial_block_manifests_match = all(
        len({manifests[name][round_index]["block_manifest"] for name in ARM_NAMES}) == 1
        for round_index in range(len(schedule))
    )
    prefix_equivalent = (
        all_complete
        and developer_maps_match
        and request_metadata_match
        and initial_block_manifests_match
    )
    mismatch_summary = _mismatch_summary(manifests, len(schedule))

    implicit = arms["hooks_implicit_enabled"]
    enabled = arms["hooks_explicit_enabled"]
    disabled = arms["hooks_explicit_disabled"]
    enabled_pair_delta = _distance(
        _usage(implicit, "uncached_input_tokens"), _usage(enabled, "uncached_input_tokens")
    )
    implicit_disabled_delta = _distance(
        _usage(implicit, "uncached_input_tokens"), _usage(disabled, "uncached_input_tokens")
    )
    enabled_disabled_delta = _distance(
        _usage(enabled, "uncached_input_tokens"), _usage(disabled, "uncached_input_tokens")
    )
    enabled_pair_equivalent = (
        enabled_pair_delta is not None and enabled_pair_delta <= EQUIVALENT_UNCACHED_THRESHOLD
    )
    disabled_separated = (
        implicit_disabled_delta is not None
        and enabled_disabled_delta is not None
        and implicit_disabled_delta >= SEPARATION_UNCACHED_THRESHOLD
        and enabled_disabled_delta >= SEPARATION_UNCACHED_THRESHOLD
    )
    if not prefix_equivalent:
        attribution = "blocked_by_observable_prefix_difference"
    elif enabled_pair_equivalent and disabled_separated:
        attribution = "effective_hooks_state_associated"
    elif not enabled_pair_equivalent:
        attribution = "explicit_override_or_uncontrolled_variance"
    elif not disabled_separated:
        attribution = "phase_6_5_divergence_not_reproduced"
    else:
        attribution = "unresolved"

    return {
        "mode": "shadow",
        "provider_effect_active": False,
        "schedule": {
            "design": "three_arm_williams_two_cycle_v1",
            "rounds": len(schedule),
            "task_blocks": 2,
            "positions_per_arm": 2,
        },
        "arms": arms,
        "initial_request_summary": {
            name: {
                "request_bytes": [item["request_bytes"] for item in manifests[name]],
                "block_counts": [item["block_count"] for item in manifests[name]],
                "estimated_tokens": [item["estimated_tokens"] for item in manifests[name]],
            }
            for name in ARM_NAMES
        },
        "integrity": {
            "all_measurements_complete": all_complete,
            "developer_stable_maps_match": developer_maps_match,
            "initial_request_metadata_match": request_metadata_match,
            "initial_block_manifests_match": initial_block_manifests_match,
            "observable_prefix_equivalent": prefix_equivalent,
        },
        "observable_prefix_differences": mismatch_summary,
        "cache_attribution": {
            "implicit_vs_explicit_enabled_uncached_delta": enabled_pair_delta,
            "implicit_enabled_vs_disabled_uncached_delta": implicit_disabled_delta,
            "explicit_enabled_vs_disabled_uncached_delta": enabled_disabled_delta,
            "equivalence_threshold": EQUIVALENT_UNCACHED_THRESHOLD,
            "separation_threshold": SEPARATION_UNCACHED_THRESHOLD,
            "enabled_pair_equivalent": enabled_pair_equivalent,
            "disabled_separated": disabled_separated,
            "classification": attribution,
        },
    }


def _execution_valid(name: str, round_index: int, evidence: dict[str, Any]) -> bool:
    execution = evidence.get("execution", {})
    return (
        evidence.get("model") == MODEL
        and evidence.get("repository_pin")
        == {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA}
        and evidence.get("forwarding_mutations") == 0
        and evidence.get("active_compression") == "off"
        and evidence.get("shadow_compression") is False
        and evidence.get("codex_user_config_loaded") is True
        and evidence.get("workspace_instruction_profile") == "none"
        and evidence.get("developer_instruction_profile") == "none"
        and evidence.get("codex_enabled_features") == sorted(ARM_ENABLED[name])
        and evidence.get("codex_disabled_features") == sorted(ARM_DISABLED[name])
        and evidence.get("codex_ephemeral") is True
        and evidence.get("codex_sandbox_profile") == "read_only"
        and evidence.get("codex_approval_policy") == "never"
        and evidence.get("search_pattern_start") == round_index // 3
        and evidence.get("experiment_round") == round_index
        and evidence.get("schedule_position") == balanced_schedule()[round_index].index(name)
        and execution.get("sessions_completed") == 1
        and execution.get("sessions_return_code_zero") == 1
        and execution.get("sessions_return_code_nonzero") == 0
        and execution.get("sessions_timed_out") == 0
    )


def build_report(
    executions: dict[str, list[dict[str, Any]]], analysis: dict[str, Any], codex_version: str
) -> dict[str, Any]:
    rounds = len(balanced_schedule())
    execution_summary: dict[str, Any] = {}
    executions_complete = True
    for name in ARM_NAMES:
        valid = len(executions.get(name, [])) == rounds and all(
            _execution_valid(name, index, evidence)
            for index, evidence in enumerate(executions.get(name, []))
        )
        executions_complete &= valid
        execution_summary[name] = {
            "runs_requested": rounds,
            "runs_completed": len(executions.get(name, [])),
            "configuration_and_schedule_contract_valid": valid,
        }
    evidence_complete = executions_complete and analysis["integrity"][
        "all_measurements_complete"
    ]
    classification = analysis["cache_attribution"]["classification"]
    decision = classification if evidence_complete else "insufficient_attribution_evidence"
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "6.6",
        "status": "completed" if executions_complete else "completed_with_execution_gaps",
        "mode": "shadow",
        "workspace_class": "public_controlled",
        "repository_pin": {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA},
        "model": MODEL,
        "codex_version": codex_version,
        "execution": execution_summary,
        "attribution": analysis,
        "gate": {
            "evidence_complete": evidence_complete,
            "observable_prefix_equivalent": analysis["integrity"]["observable_prefix_equivalent"],
            "cache_key_value_observed": False,
            "task_quality_evaluated": False,
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
            "cache_key_values_observed": False,
            "configuration_contents_in_report": False,
            "authentication_material_copied": False,
        },
        "limitations": [
            "Association with hooks state does not prove the provider cache-key construction.",
            "Only the first request's complete observable prefix is compared exactly.",
            "Provider-managed cache behavior can vary independently of visible request structure.",
            "No ordinary Tracepress instruction or provider request was modified.",
        ],
    }


def _display(value: object) -> str:
    if value is None:
        return "—"
    if isinstance(value, float):
        return f"{value:.4f}"
    return str(value)


def markdown(report: dict[str, Any]) -> str:
    analysis = report["attribution"]
    cache = analysis["cache_attribution"]
    lines = [
        "# TRACEPRESS_CACHE_KEY_ATTRIBUTION_019",
        "",
        "Phase 6.6 metadata-only cache-key attribution Shadow study.",
        "",
        f"Decision: **{report['gate']['decision']}**.",
        "",
        "## Arms",
        "",
        "| Arm | Runs | Requests | Input | Cached | Uncached |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for name in ARM_NAMES:
        arm = analysis["arms"][name]
        usage = arm["provider_usage"]
        lines.append(
            f"| `{name}` | {arm['runs_observed']} | {arm['requests_observed']} | "
            f"{_display(usage['input_tokens']['value'])} | "
            f"{_display(usage['cached_input_tokens']['value'])} | "
            f"{_display(usage['uncached_input_tokens']['value'])} |"
        )
    integrity = analysis["integrity"]
    differences = analysis["observable_prefix_differences"]
    lines.extend(
        [
            "",
            "## Observable-prefix gate",
            "",
            f"Developer maps / request metadata / complete initial block manifests match: **{integrity['developer_stable_maps_match']}/{integrity['initial_request_metadata_match']}/{integrity['initial_block_manifests_match']}**.",
            f"Observable prefix equivalent: **{integrity['observable_prefix_equivalent']}**.",
            f"Request metadata mismatch rounds: **{json.dumps(differences['request_metadata_mismatch_rounds'], sort_keys=True)}**.",
            "",
            "| Divergent initial block | Role | Origin | Rounds | Fields |",
            "|---|---|---|---:|---|",
        ]
    )
    for block in differences["divergent_block_classes"]:
        lines.append(
            f"| `{block['semantic_path'] or 'unavailable'}` (`{block['kind']}`) | "
            f"`{block['role']}` | `{block['origin']}` | {block['rounds_divergent']} | "
            f"`{','.join(block['mismatched_fields'])}` |"
        )
    lines.extend(
        [
            "",
            "## Cache attribution",
            "",
            f"Implicit vs explicit enabled uncached delta: **{_display(cache['implicit_vs_explicit_enabled_uncached_delta'])}**.",
            f"Implicit enabled vs disabled uncached delta: **{_display(cache['implicit_enabled_vs_disabled_uncached_delta'])}**.",
            f"Explicit enabled vs disabled uncached delta: **{_display(cache['explicit_enabled_vs_disabled_uncached_delta'])}**.",
            f"Classification: **{cache['classification']}**.",
            "",
            "No cache-key value was observed or persisted. This study attributes only behavior visible through controlled configuration and aggregate provider usage.",
            "",
            "Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, fingerprint, cache-key value, configuration content, authentication material, session id, or request id is in this report.",
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


def load_evidence(
    root: Path,
) -> tuple[dict[str, list[Path]], dict[str, list[dict[str, Any]]]]:
    rounds = len(balanced_schedule())
    databases = {
        name: [root / f"{name}-{index}.sqlite3" for index in range(rounds)]
        for name in ARM_NAMES
    }
    executions = {
        name: [
            json.loads((root / f"{name}-{index}.json").read_text(encoding="utf-8"))
            for index in range(rounds)
        ]
        for name in ARM_NAMES
    }
    return databases, executions


def run_live(
    repo_root: Path, root: Path, timeout: int
) -> tuple[dict[str, list[Path]], dict[str, list[dict[str, Any]]]]:
    workload = repo_root / "scripts/characterize_public_provider_native_search_001.py"
    databases = {name: [] for name in ARM_NAMES}
    executions = {name: [] for name in ARM_NAMES}
    environment = os.environ.copy()
    for round_index, order in enumerate(balanced_schedule()):
        for position, name in enumerate(order):
            database = root / f"{name}-{round_index}.sqlite3"
            output_json = root / f"{name}-{round_index}.json"
            output_md = root / f"{name}-{round_index}.md"
            command = [
                sys.executable,
                str(workload),
                "--repo-root",
                str(repo_root),
                "--sessions",
                "1",
                "--search-pattern-start",
                str(round_index // 3),
                "--codex-ephemeral",
                "--experiment-round",
                str(round_index),
                "--schedule-position",
                str(position),
                "--timeout",
                str(timeout),
                "--metadata-database-output",
                str(database),
                "--output-json",
                str(output_json),
                "--output-md",
                str(output_md),
            ]
            for feature in ARM_ENABLED[name]:
                command.extend(["--enable-codex-feature", feature])
            for feature in ARM_DISABLED[name]:
                command.extend(["--disable-codex-feature", feature])
            subprocess.run(
                command,
                cwd=repo_root,
                env=environment,
                check=True,
                capture_output=True,
                text=True,
                timeout=timeout + 180,
            )
            databases[name].append(database)
            executions[name].append(json.loads(output_json.read_text(encoding="utf-8")))
    return databases, executions


def main() -> int:
    options = parse_args()
    repo_root = options.repo_root.resolve()
    version = options.codex_version or discover_codex_version()
    if options.arm_root is not None:
        databases, executions = load_evidence(options.arm_root)
        result = analyze(databases)
        report = build_report(executions, result, version)
    else:
        with tempfile.TemporaryDirectory(prefix="tracepress-cache-key-019-", dir="/tmp") as raw:
            databases, executions = run_live(repo_root, Path(raw), options.timeout)
            result = analyze(databases)
            report = build_report(executions, result, version)
    write_report(report, options.output_json, options.output_md)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
