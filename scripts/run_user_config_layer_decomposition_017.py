#!/usr/bin/env python3
"""Decompose user-config-associated instruction layers in controlled shadow arms."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from typing import Any


ARM_NAMES = (
    "user_config_a",
    "user_config_b",
    "no_plugins",
    "no_memories",
    "no_hooks",
    "no_extensions",
    "ignore_user_config",
)
ARM_FEATURES: dict[str, tuple[str, ...]] = {
    "user_config_a": (),
    "user_config_b": (),
    "no_plugins": ("plugins",),
    "no_memories": ("memories",),
    "no_hooks": ("hooks",),
    "no_extensions": ("plugins", "memories", "hooks"),
    "ignore_user_config": (),
}
EXPERIMENT_ID = "user-config-layer-decomposition-017"
RIPGREP_REPOSITORY = "BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"
CACHE_AA_RELATIVE_THRESHOLD = 0.10
ATTRIBUTION_METRICS = (
    "common_all_arms_tokens_per_request",
    "plugins_removed_tokens_per_request",
    "plugins_added_tokens_per_request",
    "memories_removed_tokens_per_request",
    "memories_added_tokens_per_request",
    "hooks_removed_tokens_per_request",
    "hooks_added_tokens_per_request",
    "combined_removed_tokens_per_request",
    "combined_added_tokens_per_request",
    "combined_removal_explained_by_independent_layers_tokens_per_request",
    "combined_interaction_only_removed_tokens_per_request",
    "configured_no_extensions_only_tokens_per_request",
    "ignored_config_only_tokens_per_request",
)


def _load_phase_6_3_module() -> Any:
    path = Path(__file__).with_name("run_instruction_source_attribution_016.py")
    spec = importlib.util.spec_from_file_location("instruction_source_attribution_016", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load Phase 6.3 attribution reader")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PHASE_6_3 = _load_phase_6_3_module()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--rounds", type=int, default=3, choices=range(1, 6))
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


def interleaved_schedule(rounds: int) -> list[tuple[str, ...]]:
    if rounds < 1:
        raise ValueError("at least one round is required")
    remainder = list(ARM_NAMES[2:])
    schedule: list[tuple[str, ...]] = []
    for round_index in range(rounds):
        rotated = remainder[round_index % len(remainder) :] + remainder[: round_index % len(remainder)]
        pair = ["user_config_a", "user_config_b"]
        if round_index % 2:
            pair.reverse()
        insertion = round_index % (len(rotated) + 1)
        order = rotated[:insertion] + pair + rotated[insertion:]
        schedule.append(tuple(order))
    return schedule


def _metric(value: int | float | None, source: str = "provider_reported") -> dict[str, Any]:
    return {"value": value, "source": source if value is not None else "unavailable"}


def _sum_usage(parts: list[dict[str, Any]], key: str) -> int | None:
    values = [part["provider_usage"][key]["value"] for part in parts]
    if any(value is None for value in values):
        return None
    return sum(int(value) for value in values)


def _aggregate_arm(databases: list[Path]) -> tuple[dict[str, Any], dict[bytes, int]]:
    if not databases:
        raise ValueError("each arm requires at least one database")
    analyzed = [PHASE_6_3._analyze_arm(database) for database in databases]
    parts = [part for part, _stable in analyzed]
    stable_parts = [stable for _part, stable in analyzed]
    measurement_complete = all(part["measurement_complete"] for part in parts)
    common = set(stable_parts[0])
    for stable in stable_parts[1:]:
        common &= set(stable)
    weights_consistent = all(
        len({stable[identity] for stable in stable_parts}) == 1 for identity in common
    )
    stable = (
        {identity: stable_parts[0][identity] for identity in common}
        if measurement_complete and weights_consistent
        else {}
    )
    input_tokens = _sum_usage(parts, "input_tokens")
    cached_tokens = _sum_usage(parts, "cached_input_tokens")
    usage = {
        "input_tokens": _metric(input_tokens),
        "cached_input_tokens": _metric(cached_tokens),
        "uncached_input_tokens": _metric(_sum_usage(parts, "uncached_input_tokens")),
        "cache_ratio": _metric(
            cached_tokens / input_tokens
            if cached_tokens is not None and input_tokens not in (None, 0)
            else None
        ),
        "output_tokens": _metric(_sum_usage(parts, "output_tokens")),
        "reasoning_tokens": _metric(_sum_usage(parts, "reasoning_tokens")),
    }
    estimated_totals = [part["developer_estimated_tokens_total"]["value"] for part in parts]
    total_estimated = (
        sum(int(value) for value in estimated_totals)
        if measurement_complete and all(value is not None for value in estimated_totals)
        else None
    )
    return (
        {
            "runs_observed": len(databases),
            "requests_observed": sum(int(part["requests_observed"]) for part in parts),
            "complete_observations": sum(
                int(part["complete_observations"]) for part in parts
            ),
            "measurement_complete": measurement_complete and weights_consistent,
            "stable_weight_consistent": weights_consistent,
            "developer_text_blocks": (
                sum(int(part["developer_text_blocks"]) for part in parts)
                if measurement_complete
                else None
            ),
            "developer_estimated_tokens_total": _metric(
                total_estimated, "locally_estimated"
            ),
            "stable_block_classes": len(stable) if measurement_complete else None,
            "stable_tokens_per_request": (
                sum(stable.values()) if measurement_complete and weights_consistent else None
            ),
            "provider_usage": usage,
        },
        stable,
    )


def decompose_arms(databases: dict[str, list[Path]]) -> dict[str, Any]:
    if set(databases) != set(ARM_NAMES):
        raise ValueError("decomposition requires the exact controlled arm set")
    arms: dict[str, dict[str, Any]] = {}
    stable: dict[str, dict[bytes, int]] = {}
    for name in ARM_NAMES:
        arms[name], stable[name] = _aggregate_arm(databases[name])

    all_complete = all(arms[name]["measurement_complete"] for name in ARM_NAMES)
    baseline_match = stable["user_config_a"] == stable["user_config_b"] if all_complete else False
    common = set(stable[ARM_NAMES[0]])
    for name in ARM_NAMES[1:]:
        common &= set(stable[name])
    common_weight_consistent = all(
        len({stable[name][identity] for name in ARM_NAMES}) == 1 for identity in common
    )
    eligible = all_complete and baseline_match and common_weight_consistent
    baseline = stable["user_config_a"]
    baseline_a_only = set(stable["user_config_a"]) - set(stable["user_config_b"])
    baseline_b_only = set(stable["user_config_b"]) - set(stable["user_config_a"])

    def delta(arm: str) -> tuple[set[bytes], set[bytes]]:
        return set(baseline) - set(stable[arm]), set(stable[arm]) - set(baseline)

    def token_sum(source: str, identities: set[bytes]) -> int | None:
        if not eligible:
            return None
        return sum(stable[source][identity] for identity in identities)

    removals: dict[str, set[bytes]] = {}
    additions: dict[str, set[bytes]] = {}
    for layer, arm in (
        ("plugins", "no_plugins"),
        ("memories", "no_memories"),
        ("hooks", "no_hooks"),
        ("combined", "no_extensions"),
    ):
        removals[layer], additions[layer] = delta(arm)
    independent_union = removals["plugins"] | removals["memories"] | removals["hooks"]
    explained_combined = removals["combined"] & independent_union
    interaction_only = removals["combined"] - independent_union
    no_extensions = stable["no_extensions"]
    ignored = stable["ignore_user_config"]

    uncached_a = arms["user_config_a"]["provider_usage"]["uncached_input_tokens"]["value"]
    uncached_b = arms["user_config_b"]["provider_usage"]["uncached_input_tokens"]["value"]
    aa_relative_delta: float | None = None
    if uncached_a is not None and uncached_b is not None:
        midpoint = (int(uncached_a) + int(uncached_b)) / 2
        aa_relative_delta = abs(int(uncached_a) - int(uncached_b)) / max(midpoint, 1)
    baseline_uncached = (
        (int(uncached_a) + int(uncached_b)) / 2
        if uncached_a is not None and uncached_b is not None
        else None
    )
    null_uncached = arms["no_hooks"]["provider_usage"]["uncached_input_tokens"]["value"]
    null_uncached_relative_delta: float | None = None
    if baseline_uncached is not None and null_uncached is not None:
        null_midpoint = (baseline_uncached + int(null_uncached)) / 2
        null_uncached_relative_delta = abs(baseline_uncached - int(null_uncached)) / max(
            null_midpoint, 1
        )
    input_a = arms["user_config_a"]["provider_usage"]["input_tokens"]["value"]
    input_b = arms["user_config_b"]["provider_usage"]["input_tokens"]["value"]
    null_input = arms["no_hooks"]["provider_usage"]["input_tokens"]["value"]
    null_input_relative_delta: float | None = None
    if input_a is not None and input_b is not None and null_input is not None:
        baseline_input = (int(input_a) + int(input_b)) / 2
        null_input_midpoint = (baseline_input + int(null_input)) / 2
        null_input_relative_delta = abs(baseline_input - int(null_input)) / max(
            null_input_midpoint, 1
        )
    null_structural_match = stable["no_hooks"] == baseline if all_complete else False

    attribution: dict[str, int | None] = {
        "common_all_arms_tokens_per_request": token_sum("user_config_a", common),
    }
    for layer, arm in (
        ("plugins", "no_plugins"),
        ("memories", "no_memories"),
        ("hooks", "no_hooks"),
        ("combined", "no_extensions"),
    ):
        attribution[f"{layer}_removed_tokens_per_request"] = token_sum(
            "user_config_a", removals[layer]
        )
        attribution[f"{layer}_added_tokens_per_request"] = token_sum(
            arm, additions[layer]
        )
    attribution.update(
        {
            "combined_removal_explained_by_independent_layers_tokens_per_request": token_sum(
                "user_config_a", explained_combined
            ),
            "combined_interaction_only_removed_tokens_per_request": token_sum(
                "user_config_a", interaction_only
            ),
            "configured_no_extensions_only_tokens_per_request": token_sum(
                "no_extensions", set(no_extensions) - set(ignored)
            ),
            "ignored_config_only_tokens_per_request": token_sum(
                "ignore_user_config", set(ignored) - set(no_extensions)
            ),
        }
    )
    return {
        "mode": "shadow",
        "provider_effect_active": False,
        "arms": arms,
        "attribution": attribution,
        "integrity": {
            "all_arms_complete": all_complete,
            "baseline_aa_structural_match": baseline_match,
            "common_weight_consistent": common_weight_consistent,
        },
        "baseline_aa": {
            "a_only_tokens_per_request": (
                sum(stable["user_config_a"][identity] for identity in baseline_a_only)
                if all_complete
                else None
            ),
            "b_only_tokens_per_request": (
                sum(stable["user_config_b"][identity] for identity in baseline_b_only)
                if all_complete
                else None
            ),
        },
        "cache_aa": {
            "uncached_input_relative_delta": aa_relative_delta,
            "threshold": CACHE_AA_RELATIVE_THRESHOLD,
            "within_threshold": (
                aa_relative_delta <= CACHE_AA_RELATIVE_THRESHOLD
                if aa_relative_delta is not None
                else False
            ),
            "structural_null_control_match": null_structural_match,
            "structural_null_control_uncached_relative_delta": null_uncached_relative_delta,
            "structural_null_control_input_relative_delta": null_input_relative_delta,
            "structural_null_control_within_threshold": (
                null_structural_match
                and null_uncached_relative_delta is not None
                and null_uncached_relative_delta <= CACHE_AA_RELATIVE_THRESHOLD
            ),
        },
    }


def _execution_contract_valid(name: str, round_index: int, evidence: dict[str, Any]) -> bool:
    execution = evidence.get("execution", {})
    expected_position = interleaved_schedule(round_index + 1)[round_index].index(name)
    return (
        evidence.get("model") == MODEL
        and evidence.get("repository_pin")
        == {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA}
        and evidence.get("forwarding_mutations") == 0
        and evidence.get("active_compression") == "off"
        and evidence.get("shadow_compression") is False
        and evidence.get("codex_user_config_loaded") is (name != "ignore_user_config")
        and evidence.get("workspace_instruction_profile") == "none"
        and evidence.get("developer_instruction_profile") == "none"
        and evidence.get("codex_disabled_features") == sorted(ARM_FEATURES[name])
        and evidence.get("codex_ephemeral") is True
        and evidence.get("codex_sandbox_profile") == "read_only"
        and evidence.get("codex_approval_policy") == "never"
        and evidence.get("search_pattern_start") == round_index
        and evidence.get("experiment_round") == round_index
        and evidence.get("schedule_position") == expected_position
        and execution.get("sessions_completed") == 1
        and execution.get("sessions_return_code_zero") == 1
        and execution.get("sessions_return_code_nonzero") == 0
        and execution.get("sessions_timed_out") == 0
    )


def build_report(
    executions: dict[str, list[dict[str, Any]]],
    decomposition: dict[str, Any],
    rounds: int,
    codex_version_value: str,
) -> dict[str, Any]:
    if set(executions) != set(ARM_NAMES):
        raise ValueError("execution evidence requires the exact controlled arm set")
    execution_summary: dict[str, Any] = {}
    executions_complete = True
    for name in ARM_NAMES:
        evidence = executions[name]
        valid = len(evidence) == rounds and all(
            _execution_contract_valid(name, round_index, run)
            for round_index, run in enumerate(evidence)
        )
        executions_complete &= valid
        execution_summary[name] = {
            "runs_requested": rounds,
            "runs_completed": len(evidence),
            "configuration_contract_valid": valid,
        }
    integrity = decomposition["integrity"]
    structural_complete = (
        executions_complete
        and integrity["all_arms_complete"]
        and integrity["baseline_aa_structural_match"]
        and integrity["common_weight_consistent"]
    )
    cache_aa_passed = decomposition["cache_aa"]["within_threshold"]
    null_control_passed = decomposition["cache_aa"][
        "structural_null_control_within_threshold"
    ]
    decision = (
        "structural_decomposition_complete_policy_blocked"
        if structural_complete
        else "insufficient_decomposition_evidence"
    )
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "6.4",
        "status": "completed" if executions_complete else "completed_with_execution_gaps",
        "mode": "shadow",
        "workspace_class": "public_controlled",
        "repository_pin": {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA},
        "model": MODEL,
        "codex_version": codex_version_value,
        "rounds": rounds,
        "schedule": "interleaved_round_robin_v1",
        "execution": execution_summary,
        "layer_decomposition": decomposition,
        "gate": {
            "evidence_complete": structural_complete,
            "baseline_aa_structural_match": integrity["baseline_aa_structural_match"],
            "provider_cache_aa_within_threshold": decomposition["cache_aa"]["within_threshold"],
            "provider_cache_null_control_within_threshold": null_control_passed,
            "provider_comparison_reliable": cache_aa_passed and null_control_passed,
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
            "identities_in_report": False,
            "configuration_contents_in_report": False,
            "authentication_material_copied": False,
        },
        "limitations": [
            "Feature ablation flags measure runtime composition effects, not literal config-file text.",
            "Layer effects can interact and are not assumed additive.",
            "Provider cache comparisons are descriptive unless the A/A uncached-input gate passes.",
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
    decomposition = report["layer_decomposition"]
    lines = [
        "# TRACEPRESS_USER_CONFIG_LAYER_DECOMPOSITION_017",
        "",
        "Phase 6.4 controlled public cache-aware layer decomposition. Shadow only.",
        "",
        f"Decision: **{report['gate']['decision']}**.",
        "",
        "## Arms",
        "",
        "| Arm | Runs | Requests | Stable tokens/request | Input | Cached | Uncached |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for name in ARM_NAMES:
        arm = decomposition["arms"][name]
        usage = arm["provider_usage"]
        lines.append(
            f"| `{name}` | {arm['runs_observed']} | {arm['requests_observed']} | "
            f"{_display(arm['stable_tokens_per_request'])} | "
            f"{_display(usage['input_tokens']['value'])} | "
            f"{_display(usage['cached_input_tokens']['value'])} | "
            f"{_display(usage['uncached_input_tokens']['value'])} |"
        )
    lines.extend(["", "## Layer attribution", "", "| Metric | Tokens/request |", "|---|---:|"])
    for key in ATTRIBUTION_METRICS:
        lines.append(f"| `{key}` | {_display(decomposition['attribution'][key])} |")
    cache = decomposition["cache_aa"]
    structural_aa = decomposition["baseline_aa"]
    lines.extend(
        [
            "",
            "## Cache A/A",
            "",
            f"Structural A-only/B-only tokens per request: **{_display(structural_aa['a_only_tokens_per_request'])}/{_display(structural_aa['b_only_tokens_per_request'])}**.",
            "",
            f"Uncached-input relative delta: **{_display(cache['uncached_input_relative_delta'])}**. Threshold: **{cache['threshold']}**. Within threshold: **{cache['within_threshold']}**.",
            "",
            f"Structurally null `no_hooks` uncached/input relative deltas: **{_display(cache['structural_null_control_uncached_relative_delta'])}/{_display(cache['structural_null_control_input_relative_delta'])}**. Uncached within threshold: **{cache['structural_null_control_within_threshold']}**.",
            "",
            "Provider usage remains descriptive and is not attributed savings.",
            "",
            "Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, identity, configuration content, authentication material, session id, or request id is in this report.",
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


def load_arm_evidence(
    root: Path, rounds: int
) -> tuple[dict[str, list[Path]], dict[str, list[dict[str, Any]]]]:
    databases = {
        name: [root / f"{name}-{round_index}.sqlite3" for round_index in range(rounds)]
        for name in ARM_NAMES
    }
    executions = {
        name: [
            json.loads((root / f"{name}-{round_index}.json").read_text(encoding="utf-8"))
            for round_index in range(rounds)
        ]
        for name in ARM_NAMES
    }
    return databases, executions


def run_live_arms(
    repo_root: Path, root: Path, rounds: int, timeout: int
) -> tuple[dict[str, list[Path]], dict[str, list[dict[str, Any]]]]:
    workload = repo_root / "scripts/characterize_public_provider_native_search_001.py"
    databases = {name: [] for name in ARM_NAMES}
    executions = {name: [] for name in ARM_NAMES}
    environment = os.environ.copy()
    for round_index, order in enumerate(interleaved_schedule(rounds)):
        for schedule_position, name in enumerate(order):
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
                str(round_index),
                "--codex-ephemeral",
                "--experiment-round",
                str(round_index),
                "--schedule-position",
                str(schedule_position),
                "--timeout",
                str(timeout),
                "--metadata-database-output",
                str(database),
                "--output-json",
                str(output_json),
                "--output-md",
                str(output_md),
            ]
            if name == "ignore_user_config":
                command.append("--ignore-user-config")
            for feature in ARM_FEATURES[name]:
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
        databases, executions = load_arm_evidence(options.arm_root, options.rounds)
        decomposition = decompose_arms(databases)
        report = build_report(executions, decomposition, options.rounds, version)
    else:
        with tempfile.TemporaryDirectory(prefix="tracepress-config-layers-017-", dir="/tmp") as raw:
            databases, executions = run_live_arms(
                repo_root, Path(raw), options.rounds, options.timeout
            )
            decomposition = decompose_arms(databases)
            report = build_report(executions, decomposition, options.rounds, version)
    write_report(report, options.output_json, options.output_md)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
