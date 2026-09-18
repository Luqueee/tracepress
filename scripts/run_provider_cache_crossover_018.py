#!/usr/bin/env python3
"""Run a fully position-balanced provider-cache crossover in Shadow mode."""

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
    "user_config_control",
    "no_hooks_null",
    "no_extensions_treatment",
)
ARM_FEATURES: dict[str, tuple[str, ...]] = {
    "user_config_control": (),
    "no_hooks_null": ("hooks",),
    "no_extensions_treatment": ("plugins", "memories", "hooks"),
}
EXPERIMENT_ID = "provider-cache-crossover-018"
RIPGREP_REPOSITORY = "BurntSushi/ripgrep"
RIPGREP_SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
MODEL = "gpt-5.6-luna"
NULL_UNCACHED_AGGREGATE_THRESHOLD = 0.10
NULL_UNCACHED_POSITION_THRESHOLD = 0.20
NULL_INPUT_TOTAL_THRESHOLD = 0.02


def _load_phase_6_4_module() -> Any:
    path = Path(__file__).with_name("run_user_config_layer_decomposition_017.py")
    spec = importlib.util.spec_from_file_location("user_config_layer_decomposition_017", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load Phase 6.4 attribution reader")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PHASE_6_4 = _load_phase_6_4_module()


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


def balanced_crossover_schedule() -> tuple[tuple[str, ...], ...]:
    control, null, treatment = ARM_NAMES
    return (
        (control, null, treatment),
        (null, treatment, control),
        (treatment, control, null),
        (control, treatment, null),
        (treatment, null, control),
        (null, control, treatment),
    )


def _distance(left: int | float | None, right: int | float | None) -> float | None:
    if left is None or right is None:
        return None
    midpoint = (float(left) + float(right)) / 2
    return abs(float(left) - float(right)) / max(midpoint, 1.0)


def _change(treatment: int | float | None, control: int | float | None) -> float | None:
    if treatment is None or control in (None, 0):
        return None
    return (float(treatment) - float(control)) / float(control)


def _usage_values(arm: dict[str, Any]) -> dict[str, int | None]:
    return {
        key: arm["provider_usage"][key]["value"]
        for key in (
            "input_tokens",
            "cached_input_tokens",
            "uncached_input_tokens",
            "output_tokens",
            "reasoning_tokens",
        )
    }


def _sum_values(values: list[int | None]) -> int | None:
    if any(value is None for value in values):
        return None
    return sum(int(value) for value in values)


def analyze_crossover(databases: dict[str, list[Path]]) -> dict[str, Any]:
    if set(databases) != set(ARM_NAMES):
        raise ValueError("crossover requires the exact controlled arm set")
    schedule = balanced_crossover_schedule()
    if any(len(databases[name]) != len(schedule) for name in ARM_NAMES):
        raise ValueError("every arm requires one database per crossover round")

    arms: dict[str, dict[str, Any]] = {}
    stable: dict[str, dict[bytes, int]] = {}
    per_run: dict[str, list[dict[str, Any]]] = {}
    for name in ARM_NAMES:
        arms[name], stable[name] = PHASE_6_4._aggregate_arm(databases[name])
        per_run[name] = [
            PHASE_6_4.PHASE_6_3._analyze_arm(database)[0]
            for database in databases[name]
        ]

    all_complete = all(arms[name]["measurement_complete"] for name in ARM_NAMES)
    control_null_match = (
        stable["user_config_control"] == stable["no_hooks_null"]
        if all_complete
        else False
    )
    common = set(stable[ARM_NAMES[0]])
    for name in ARM_NAMES[1:]:
        common &= set(stable[name])
    common_weight_consistent = all(
        len({stable[name][identity] for name in ARM_NAMES}) == 1 for identity in common
    )
    structural_complete = all_complete and control_null_match and common_weight_consistent

    position_usage: dict[str, dict[str, dict[str, int | None]]] = {
        name: {} for name in ARM_NAMES
    }
    for name in ARM_NAMES:
        for position in range(len(ARM_NAMES)):
            round_indexes = [
                round_index
                for round_index, order in enumerate(schedule)
                if order[position] == name
            ]
            position_usage[name][str(position)] = {
                key: _sum_values(
                    [
                        _usage_values(per_run[name][round_index])[key]
                        for round_index in round_indexes
                    ]
                )
                for key in _usage_values(per_run[name][0])
            }

    control_usage = _usage_values(arms["user_config_control"])
    null_usage = _usage_values(arms["no_hooks_null"])
    treatment_usage = _usage_values(arms["no_extensions_treatment"])
    null_position_uncached_deltas = {
        str(position): _distance(
            position_usage["user_config_control"][str(position)][
                "uncached_input_tokens"
            ],
            position_usage["no_hooks_null"][str(position)]["uncached_input_tokens"],
        )
        for position in range(len(ARM_NAMES))
    }
    null_uncached_aggregate = _distance(
        control_usage["uncached_input_tokens"], null_usage["uncached_input_tokens"]
    )
    null_input_total = _distance(
        control_usage["input_tokens"], null_usage["input_tokens"]
    )
    max_position_delta = (
        max(float(value) for value in null_position_uncached_deltas.values() if value is not None)
        if all(value is not None for value in null_position_uncached_deltas.values())
        else None
    )
    total_input_reliable = (
        structural_complete
        and null_input_total is not None
        and null_input_total <= NULL_INPUT_TOTAL_THRESHOLD
    )
    cache_split_reliable = (
        structural_complete
        and null_uncached_aggregate is not None
        and null_uncached_aggregate <= NULL_UNCACHED_AGGREGATE_THRESHOLD
        and max_position_delta is not None
        and max_position_delta <= NULL_UNCACHED_POSITION_THRESHOLD
    )
    provider_reliable = total_input_reliable and cache_split_reliable

    control_keys = set(stable["user_config_control"])
    treatment_keys = set(stable["no_extensions_treatment"])
    return {
        "mode": "shadow",
        "provider_effect_active": False,
        "schedule": {
            "design": "three_arm_williams_two_cycle_v1",
            "rounds": len(schedule),
            "task_blocks": 2,
            "rounds_per_task": 3,
            "positions_per_arm": 2,
            "positions_per_arm_per_task": 1,
            "directed_transitions_per_pair": 2,
        },
        "arms": arms,
        "position_usage": position_usage,
        "integrity": {
            "all_arms_complete": all_complete,
            "control_null_structural_match": control_null_match,
            "common_weight_consistent": common_weight_consistent,
        },
        "structural_treatment": {
            "control_stable_tokens_per_request": arms["user_config_control"][
                "stable_tokens_per_request"
            ],
            "treatment_stable_tokens_per_request": arms["no_extensions_treatment"][
                "stable_tokens_per_request"
            ],
            "removed_tokens_per_request": (
                sum(stable["user_config_control"][identity] for identity in control_keys - treatment_keys)
                if structural_complete
                else None
            ),
            "added_tokens_per_request": (
                sum(stable["no_extensions_treatment"][identity] for identity in treatment_keys - control_keys)
                if structural_complete
                else None
            ),
        },
        "cache_controls": {
            "null_input_total_relative_delta": null_input_total,
            "null_uncached_aggregate_relative_delta": null_uncached_aggregate,
            "null_uncached_position_relative_deltas": null_position_uncached_deltas,
            "null_uncached_max_position_relative_delta": max_position_delta,
            "input_total_threshold": NULL_INPUT_TOTAL_THRESHOLD,
            "uncached_aggregate_threshold": NULL_UNCACHED_AGGREGATE_THRESHOLD,
            "uncached_position_threshold": NULL_UNCACHED_POSITION_THRESHOLD,
            "total_input_comparison_reliable": total_input_reliable,
            "cache_split_comparison_reliable": cache_split_reliable,
            "provider_comparison_reliable": provider_reliable,
        },
        "treatment_effect": {
            "input_total_relative_change": _change(
                treatment_usage["input_tokens"], control_usage["input_tokens"]
            ),
            "cached_input_relative_change": _change(
                treatment_usage["cached_input_tokens"],
                control_usage["cached_input_tokens"],
            ),
            "uncached_input_relative_change": _change(
                treatment_usage["uncached_input_tokens"],
                control_usage["uncached_input_tokens"],
            ),
            "output_relative_change": _change(
                treatment_usage["output_tokens"], control_usage["output_tokens"]
            ),
            "reasoning_relative_change": _change(
                treatment_usage["reasoning_tokens"],
                control_usage["reasoning_tokens"],
            ),
            "interpretation": (
                "eligible_for_quality_gated_follow_up"
                if provider_reliable
                else "blocked_by_cache_controls"
            ),
        },
    }


def _execution_valid(name: str, round_index: int, evidence: dict[str, Any]) -> bool:
    schedule = balanced_crossover_schedule()
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
        and evidence.get("codex_disabled_features") == sorted(ARM_FEATURES[name])
        and evidence.get("codex_ephemeral") is True
        and evidence.get("codex_sandbox_profile") == "read_only"
        and evidence.get("codex_approval_policy") == "never"
        and evidence.get("search_pattern_start") == round_index // 3
        and evidence.get("experiment_round") == round_index
        and evidence.get("schedule_position") == schedule[round_index].index(name)
        and execution.get("sessions_completed") == 1
        and execution.get("sessions_return_code_zero") == 1
        and execution.get("sessions_return_code_nonzero") == 0
        and execution.get("sessions_timed_out") == 0
    )


def build_report(
    executions: dict[str, list[dict[str, Any]]],
    analysis: dict[str, Any],
    codex_version_value: str,
) -> dict[str, Any]:
    if set(executions) != set(ARM_NAMES):
        raise ValueError("execution evidence requires the exact controlled arm set")
    rounds = len(balanced_crossover_schedule())
    execution_summary: dict[str, Any] = {}
    executions_complete = True
    for name in ARM_NAMES:
        valid = len(executions[name]) == rounds and all(
            _execution_valid(name, round_index, evidence)
            for round_index, evidence in enumerate(executions[name])
        )
        executions_complete &= valid
        execution_summary[name] = {
            "runs_requested": rounds,
            "runs_completed": len(executions[name]),
            "configuration_and_schedule_contract_valid": valid,
        }
    integrity = analysis["integrity"]
    evidence_complete = (
        executions_complete
        and integrity["all_arms_complete"]
        and integrity["control_null_structural_match"]
        and integrity["common_weight_consistent"]
    )
    provider_reliable = analysis["cache_controls"]["provider_comparison_reliable"]
    if not evidence_complete:
        decision = "insufficient_crossover_evidence"
    elif provider_reliable:
        decision = "crossover_baseline_complete_policy_blocked"
    else:
        decision = "crossover_baseline_failed_provider_reliability"
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "6.5",
        "status": "completed" if executions_complete else "completed_with_execution_gaps",
        "mode": "shadow",
        "workspace_class": "public_controlled",
        "repository_pin": {"repository": RIPGREP_REPOSITORY, "commit_sha": RIPGREP_SHA},
        "model": MODEL,
        "codex_version": codex_version_value,
        "execution": execution_summary,
        "crossover": analysis,
        "gate": {
            "evidence_complete": evidence_complete,
            "provider_comparison_reliable": provider_reliable,
            "total_input_comparison_reliable": analysis["cache_controls"][
                "total_input_comparison_reliable"
            ],
            "cache_split_comparison_reliable": analysis["cache_controls"][
                "cache_split_comparison_reliable"
            ],
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
            "identities_in_report": False,
            "configuration_contents_in_report": False,
            "authentication_material_copied": False,
        },
        "limitations": [
            "Provider comparison reliability does not establish task quality or authorize a policy.",
            "The public Search workload is narrow and results do not generalize to arbitrary tasks.",
            "Feature ablation measures runtime composition effects, not literal config-file text.",
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
    crossover = report["crossover"]
    lines = [
        "# TRACEPRESS_PROVIDER_CACHE_CROSSOVER_018",
        "",
        "Phase 6.5 fully position-balanced provider-cache crossover. Shadow only.",
        "",
        f"Decision: **{report['gate']['decision']}**.",
        "",
        "## Arms",
        "",
        "| Arm | Runs | Requests | Stable tokens/request | Input | Cached | Uncached |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for name in ARM_NAMES:
        arm = crossover["arms"][name]
        usage = arm["provider_usage"]
        lines.append(
            f"| `{name}` | {arm['runs_observed']} | {arm['requests_observed']} | "
            f"{_display(arm['stable_tokens_per_request'])} | "
            f"{_display(usage['input_tokens']['value'])} | "
            f"{_display(usage['cached_input_tokens']['value'])} | "
            f"{_display(usage['uncached_input_tokens']['value'])} |"
        )
    controls = crossover["cache_controls"]
    treatment = crossover["treatment_effect"]
    structural = crossover["structural_treatment"]
    lines.extend(
        [
            "",
            "## Structural crossover",
            "",
            f"Control/null exact stable-map match: **{crossover['integrity']['control_null_structural_match']}**.",
            f"Treatment removed/added tokens per request: **{_display(structural['removed_tokens_per_request'])}/{_display(structural['added_tokens_per_request'])}**.",
            "",
            "## Cache controls",
            "",
            f"Null total-input relative delta: **{_display(controls['null_input_total_relative_delta'])}** (threshold {controls['input_total_threshold']}).",
            f"Null aggregate uncached relative delta: **{_display(controls['null_uncached_aggregate_relative_delta'])}** (threshold {controls['uncached_aggregate_threshold']}).",
            f"Null per-position uncached deltas: **{', '.join(f'{position}={_display(value)}' for position, value in controls['null_uncached_position_relative_deltas'].items())}** (max threshold {controls['uncached_position_threshold']}).",
            f"Total-input comparison reliable: **{controls['total_input_comparison_reliable']}**. Cache-split comparison reliable: **{controls['cache_split_comparison_reliable']}**.",
            f"Provider comparison reliable: **{controls['provider_comparison_reliable']}**.",
            "",
            "## Treatment observation",
            "",
            f"Input/cached/uncached relative changes: **{_display(treatment['input_total_relative_change'])}/{_display(treatment['cached_input_relative_change'])}/{_display(treatment['uncached_input_relative_change'])}**.",
            f"Output/reasoning relative changes: **{_display(treatment['output_relative_change'])}/{_display(treatment['reasoning_relative_change'])}**.",
            f"Interpretation: **{treatment['interpretation']}**. Task quality was not evaluated, so no policy is authorized.",
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
    root: Path,
) -> tuple[dict[str, list[Path]], dict[str, list[dict[str, Any]]]]:
    rounds = len(balanced_crossover_schedule())
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
    repo_root: Path, root: Path, timeout: int
) -> tuple[dict[str, list[Path]], dict[str, list[dict[str, Any]]]]:
    workload = repo_root / "scripts/characterize_public_provider_native_search_001.py"
    databases = {name: [] for name in ARM_NAMES}
    executions = {name: [] for name in ARM_NAMES}
    environment = os.environ.copy()
    for round_index, order in enumerate(balanced_crossover_schedule()):
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
        databases, executions = load_arm_evidence(options.arm_root)
        analysis = analyze_crossover(databases)
        report = build_report(executions, analysis, version)
    else:
        with tempfile.TemporaryDirectory(prefix="tracepress-cache-crossover-018-", dir="/tmp") as raw:
            databases, executions = run_live_arms(repo_root, Path(raw), options.timeout)
            analysis = analyze_crossover(databases)
            report = build_report(executions, analysis, version)
    write_report(report, options.output_json, options.output_md)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
