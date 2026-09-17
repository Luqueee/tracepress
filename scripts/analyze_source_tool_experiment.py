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
        "source_ids_present",
        "source_sessions_linked",
        "source_emitted_bytes",
        "shadow_evaluations",
        "candidate_bytes",
        "estimated_raw_tokens",
        "estimated_candidate_tokens",
        "never_worse_accepted",
        "recovery_hint_bytes",
        "reducer_duration_us",
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
    if control_name in {"ExplicitB", "ExplicitShadow"}:
        control_name, treatment_name = treatment_name, control_name
    control, treatment = arms[control_name], arms[treatment_name]
    if len(control) != len(treatment):
        raise ValueError("unbalanced experiment arms")

    metrics = ["provider_requests", "duration_ms"] + [
        f"provider_usage.{key}" for key in USAGE_KEYS
    ]
    deltas = {metric: paired_deltas(control, treatment, metric) for metric in metrics}
    invariant_gate = all(
        left.get("agent_exit_status_class") == "success"
        and right.get("agent_exit_status_class") == "success"
        and value(left, "provider_errors") == 0
        and value(right, "provider_errors") == 0
        and value(left, "source_executions") == 1
        and value(right, "source_executions") == 1
        and ("source_ids_present" not in left or value(left, "source_ids_present") == 1)
        and ("source_ids_present" not in right or value(right, "source_ids_present") == 1)
        and ("source_sessions_linked" not in left or value(left, "source_sessions_linked") == 1)
        and ("source_sessions_linked" not in right or value(right, "source_sessions_linked") == 1)
        and value(left, "hook_rewrites") == 0
        and value(right, "hook_rewrites") == 0
        and value(left, "source_emitted_bytes") == value(right, "source_emitted_bytes")
        for left, right in zip(control, treatment)
    )
    shadow = report.get("reducer") == "explicit-shadow"
    shadow_gate = True
    source_reduction = None
    if shadow:
        raw = sum(value(row, "source_emitted_bytes") for row in treatment)
        candidate = sum(value(row, "candidate_bytes") for row in treatment)
        source_reduction = (raw - candidate) / raw if raw else 0.0
        shadow_gate = (
            all(value(row, "shadow_evaluations") == 1 for row in treatment)
            and all(value(row, "never_worse_accepted") == 1 for row in treatment)
            and source_reduction >= 0.20
            and max((value(row, "reducer_duration_us") for row in treatment), default=0) <= 50_000
        )
    return {
        "control_arm": control_name,
        "treatment_arm": treatment_name,
        "arms": {name: aggregate(arm_rows) for name, arm_rows in arms.items()},
        "paired_treatment_minus_control": deltas,
        "source_candidate_reduction": source_reduction,
        "invariant_gate": invariant_gate,
        "shadow_gate": shadow_gate if shadow else None,
        "decision": "pass" if invariant_gate and shadow_gate else "reject",
        "interpretation": (
            "candidate_only_no_forwarding_change" if shadow else "passthrough_noise_characterization"
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
            f"Invariant gate: **{str(analysis['invariant_gate']).lower()}**.",
        ]
    )
    if analysis["source_candidate_reduction"] is not None:
        lines.extend(
            [
                f"Source candidate reduction: **{analysis['source_candidate_reduction']:.2%}**.",
                f"Shadow gate: **{str(analysis['shadow_gate']).lower()}**.",
                "Agent-visible output remained raw; provider deltas are not source-reduction savings.",
            ]
        )
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
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.markdown.write_text(markdown(report, analysis), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
