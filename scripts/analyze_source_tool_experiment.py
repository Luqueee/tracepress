#!/usr/bin/env python3
"""Add paired aggregate analysis and a human-readable decision to a source experiment."""

from __future__ import annotations

import argparse
import json
import statistics
from pathlib import Path
from typing import Any


USAGE_KEYS = ("input_total", "input_cached", "input_uncached", "output", "reasoning")


def value(row: dict[str, Any], metric: str) -> int:
    if metric.startswith("provider_usage."):
        return int(row.get("provider_usage", {}).get(metric.removeprefix("provider_usage.")) or 0)
    return int(row.get(metric) or 0)


def aggregate(rows: list[dict[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {
        "runs": len(rows),
        "successful_runs": sum(row.get("agent_exit_status_class") == "success" for row in rows),
        "timed_out_runs": sum(row.get("timed_out") is True for row in rows),
    }
    for metric in (
        "provider_requests",
        "provider_errors",
        "source_executions",
        "source_successful_executions",
        "source_ids_present",
        "source_sessions_linked",
        "source_emitted_bytes",
        "source_stdout_bytes",
        "source_stderr_bytes",
        "shadow_evaluations",
        "active_evaluations",
        "forwarding_mutations",
        "fail_open_executions",
        "candidate_bytes",
        "estimated_raw_tokens",
        "estimated_candidate_tokens",
        "never_worse_accepted",
        "recovery_hint_bytes",
        "omitted_advisory_lines",
        "grouped_match_lines",
        "reducer_duration_us",
        "recovery_requests",
        "recovered_bytes",
        "tool_calls",
        "command_retries",
        "duration_ms",
    ):
        values = [value(row, metric) for row in rows]
        result[metric] = {
            "total": sum(values),
            "median": statistics.median(values) if values else 0,
        }
    result["provider_usage"] = {
        key: sum(value(row, f"provider_usage.{key}") for row in rows) for key in USAGE_KEYS
    }
    result["task_success"] = sum(row.get("task_success") is True for row in rows)
    return result


def paired_deltas(
    control: list[dict[str, Any]], treatment: list[dict[str, Any]], metric: str
) -> dict[str, Any]:
    deltas = [value(right, metric) - value(left, metric) for left, right in zip(control, treatment)]
    return {
        "values": deltas,
        "median": statistics.median(deltas) if deltas else 0,
        "min": min(deltas, default=0),
        "max": max(deltas, default=0),
        "positive": sum(delta > 0 for delta in deltas),
        "negative": sum(delta < 0 for delta in deltas),
        "zero": sum(delta == 0 for delta in deltas),
    }


def analyze(report: dict[str, Any]) -> dict[str, Any]:
    rows = report["rows"]
    arm_names = list(dict.fromkeys(row["arm"] for row in rows))
    if len(arm_names) != 2:
        raise ValueError("expected exactly two experiment arms")
    arms = {name: [row for row in rows if row["arm"] == name] for name in arm_names}
    control_name, treatment_name = arm_names
    if control_name in {"ExplicitB", "ExplicitShadow", "ExplicitActive", "ExplicitCheckShadow", "ExplicitCheckActive", "ExplicitCheckV2Shadow", "ExplicitCheckV2Active", "ExplicitClippyShadow", "ExplicitRgShadow", "ExplicitRgActive", "ExplicitGitStatusShadow", "ExplicitGitStatusActive"}:
        control_name, treatment_name = treatment_name, control_name
    control, treatment = arms[control_name], arms[treatment_name]
    if len(control) != len(treatment):
        raise ValueError("unbalanced experiment arms")

    metrics = ["provider_requests", "duration_ms"] + [
        f"provider_usage.{key}" for key in USAGE_KEYS
    ]
    deltas = {metric: paired_deltas(control, treatment, metric) for metric in metrics}
    active = report.get("reducer") in {"explicit-active", "explicit-check-active", "explicit-check-active-v2", "explicit-rg-active", "explicit-git-status-active"}
    active_fail_open = active and report.get("scenario") in {"diagnostic-failure", "ambiguous", "clean"}
    invariant_gate = all(
        left.get("agent_exit_status_class") == "success"
        and right.get("agent_exit_status_class") == "success"
        and value(left, "provider_errors") == 0
        and value(right, "provider_errors") == 0
        and value(left, "source_executions") >= 1
        and value(right, "source_executions") >= 1
        and ("source_ids_present" not in left or value(left, "source_ids_present") == value(left, "source_executions"))
        and ("source_ids_present" not in right or value(right, "source_ids_present") == value(right, "source_executions"))
        and ("source_sessions_linked" not in left or value(left, "source_sessions_linked") == value(left, "source_executions"))
        and ("source_sessions_linked" not in right or value(right, "source_sessions_linked") == value(right, "source_executions"))
        and value(left, "hook_rewrites") == 0
        and value(right, "hook_rewrites") == 0
        and (active or value(left, "source_emitted_bytes") == value(right, "source_emitted_bytes"))
        for left, right in zip(control, treatment)
    )
    shadow = report.get("reducer") in {"explicit-shadow", "explicit-check-shadow", "explicit-check-shadow-v2", "explicit-clippy-shadow", "explicit-rg-shadow", "explicit-git-status-shadow"}
    shadow_gate = True
    active_gate = True
    positive_gate = None
    source_reduction = None
    if shadow or active:
        raw = sum(
            value(row, "source_stdout_bytes") + value(row, "source_stderr_bytes")
            for row in treatment
        )
        emitted = sum(
            value(row, "source_emitted_bytes") if active else value(row, "candidate_bytes")
            for row in treatment
        )
        source_reduction = (raw - emitted) / raw if raw else 0.0
    if shadow:
        shadow_gate = (
            all(value(row, "shadow_evaluations") == 1 for row in treatment)
            and all(value(row, "never_worse_accepted") == 1 for row in treatment)
            and source_reduction >= 0.20
            and max((value(row, "reducer_duration_us") for row in treatment), default=0) <= 50_000
        )
    if active:
        control_uncached = sum(value(row, "provider_usage.input_uncached") for row in control)
        treatment_uncached = sum(value(row, "provider_usage.input_uncached") for row in treatment)
        if active_fail_open:
            active_gate = (
                all(row.get("task_success") is True for row in control + treatment)
                and all(value(row, "active_evaluations") >= 1 for row in treatment)
                and all(value(row, "forwarding_mutations") == 0 for row in treatment)
                and all(value(row, "fail_open_executions") == value(row, "active_evaluations") for row in treatment)
                and all(value(row, "never_worse_accepted") == 0 for row in treatment)
                and all(value(row, "candidate_bytes") == value(row, "source_stdout_bytes") + value(row, "source_stderr_bytes") for row in treatment)
                and all(value(row, "source_emitted_bytes") == value(row, "source_stdout_bytes") + value(row, "source_stderr_bytes") for row in treatment)
                and all(value(row, "recovery_requests") == 0 for row in treatment)
            )
        else:
            active_gate = (
                all(row.get("task_success") is True for row in control + treatment)
                and all(value(row, "active_evaluations") >= 1 for row in treatment)
                and all(value(row, "forwarding_mutations") >= 1 for row in treatment)
                and all(value(row, "fail_open_executions") == 0 for row in treatment)
                and all(
                    value(row, "never_worse_accepted") == value(row, "active_evaluations")
                    for row in treatment
                )
                and source_reduction is not None
                and source_reduction >= 0.20
            )
        if report.get("pairs", 0) >= 10:
            trajectory_gate = (
                sum(value(row, "command_retries") for row in treatment)
                <= sum(value(row, "command_retries") for row in control) + 1
                and sum(value(row, "provider_requests") for row in treatment)
                <= sum(value(row, "provider_requests") for row in control) + 1
                and sum(value(row, "tool_calls") for row in treatment)
                <= sum(value(row, "tool_calls") for row in control) + 1
            )
            positive_gate = trajectory_gate if active_fail_open else (
                treatment_uncached < control_uncached
                and deltas["provider_usage.input_uncached"]["median"] < 0
                and trajectory_gate
                and sum(value(row, "recovery_requests") for row in treatment)
                <= max(1, sum(value(row, "forwarding_mutations") for row in treatment) // 10)
            )
    return {
        "control_arm": control_name,
        "treatment_arm": treatment_name,
        "arms": {name: aggregate(arm_rows) for name, arm_rows in arms.items()},
        "paired_treatment_minus_control": deltas,
        "source_candidate_reduction": source_reduction,
        "invariant_gate": invariant_gate,
        "shadow_gate": shadow_gate if shadow else None,
        "active_gate": active_gate if active else None,
        "positive_gate": positive_gate,
        "decision": "pass" if invariant_gate and shadow_gate and active_gate and positive_gate is not False else "reject",
        "interpretation": (
            "active_fail_open_safety" if active_fail_open else ("active_provider_effect" if active else ("candidate_only_no_forwarding_change" if shadow else "passthrough_noise_characterization"))
        ),
    }


def markdown(report: dict[str, Any], analysis: dict[str, Any]) -> str:
    control = analysis["control_arm"]
    treatment = analysis["treatment_arm"]
    lines = [
        "# Tracepress explicit source-tool experiment",
        "",
        f"Decision: **{analysis['decision'].upper()}**.",
        "",
        f"Control: `{control}`. Treatment: `{treatment}`. Pairs: {report['pairs']}.",
        "",
        "| Metric | Control total | Treatment total | Paired median delta |",
        "|---|---:|---:|---:|",
    ]
    for metric in ("provider_requests", *[f"provider_usage.{key}" for key in USAGE_KEYS], "duration_ms"):
        if metric.startswith("provider_usage."):
            key = metric.removeprefix("provider_usage.")
            left = analysis["arms"][control]["provider_usage"][key]
            right = analysis["arms"][treatment]["provider_usage"][key]
        else:
            left = analysis["arms"][control][metric]["total"]
            right = analysis["arms"][treatment][metric]["total"]
        median = analysis["paired_treatment_minus_control"][metric]["median"]
        lines.append(f"| `{metric}` | {left} | {right} | {median} |")
    lines.extend(
        [
            "",
            "| Trajectory metric | Control | Treatment |",
            "|---|---:|---:|",
            f"| Task success | {analysis['arms'][control]['task_success']}/{analysis['arms'][control]['runs']} | {analysis['arms'][treatment]['task_success']}/{analysis['arms'][treatment]['runs']} |",
            f"| Tool calls | {analysis['arms'][control]['tool_calls']['total']} | {analysis['arms'][treatment]['tool_calls']['total']} |",
            f"| Command retries | {analysis['arms'][control]['command_retries']['total']} | {analysis['arms'][treatment]['command_retries']['total']} |",
            f"| Recovery requests | {analysis['arms'][control]['recovery_requests']['total']} | {analysis['arms'][treatment]['recovery_requests']['total']} |",
            f"| Raw source bytes | {analysis['arms'][control]['source_stdout_bytes']['total'] + analysis['arms'][control]['source_stderr_bytes']['total']} | {analysis['arms'][treatment]['source_stdout_bytes']['total'] + analysis['arms'][treatment]['source_stderr_bytes']['total']} |",
            f"| Emitted source bytes | {analysis['arms'][control]['source_emitted_bytes']['total']} | {analysis['arms'][treatment]['source_emitted_bytes']['total']} |",
        ]
    )
    lines.extend(
        [
            "",
            f"Invariant gate: **{str(analysis['invariant_gate']).lower()}**.",
        ]
    )
    if analysis["source_candidate_reduction"] is not None:
        lines.append(f"Source output reduction: **{analysis['source_candidate_reduction']:.2%}**.")
        if analysis["active_gate"] is not None:
            lines.extend(
                [
                    f"Active gate: **{str(analysis['active_gate']).lower()}**.",
                    f"Positive gate: **{str(analysis['positive_gate']).lower() if analysis['positive_gate'] is not None else 'pending full cohort'}**.",
                    "Treatment fail-open preserved raw output; downstream deltas are safety A/A evidence." if report.get("scenario") in {"diagnostic-failure", "ambiguous", "clean"} else "Treatment forwarded the accepted candidate; downstream deltas are active-pilot evidence.",
                ]
            )
        else:
            lines.append(f"Shadow gate: **{str(analysis['shadow_gate']).lower()}**.")
            lines.append("Agent-visible output remained raw; provider deltas are not source-reduction savings.")
    else:
        lines.append("Provider and trajectory deltas characterize A/A noise between identical arms.")
    lines.extend(
        [
            "",
            "No prompts, commands, paths, raw tool output, or provider bodies are present in this report.",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--markdown", type=Path, required=True)
    args = parser.parse_args()
    report = json.loads(args.report.read_text(encoding="utf-8"))
    analysis = analyze(report)
    report["analysis"] = analysis
    if report.get("reducer") in {"explicit-active", "explicit-check-active", "explicit-check-active-v2", "explicit-rg-active", "explicit-git-status-active"}:
        report["assignment"] = "session_level"
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.markdown.write_text(markdown(report, analysis), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
