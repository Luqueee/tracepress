#!/usr/bin/env python3
"""Check adaptive convergence of token-weighted Tracepress baseline reports."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any


MIN_SESSIONS = 40
SHARE_TOLERANCE = 0.02
RELATIVE_TOLERANCE = 0.10
QUALITY_TARGETS = {
    "analysis_coverage": 0.95,
    "correlation_coverage": 0.99,
}


def _number(value: Any) -> float | None:
    if isinstance(value, bool) or value is None:
        return None
    try:
        value = float(value)
    except (TypeError, ValueError):
        return None
    return value if math.isfinite(value) else None


def _category_share(report: dict[str, Any], section: str, name: str) -> float | None:
    for row in report.get("composition", {}).get(section, []) or []:
        if row.get("name") == name:
            return _number(row.get("token_share"))
    return 0.0


def _metric_values(report: dict[str, Any]) -> dict[str, float | None]:
    quality = report.get("quality", {})
    composition = report.get("composition", {})
    repetition = report.get("repetition", {})
    usage = report.get("provider_usage", {})
    stable_prefix = report.get("stable_prefix", {})
    distributions = report.get("distributions", {}).get("estimated_tokens", {})
    return {
        "plain_text_token_share": _category_share(report, "detected_content", "plain_text"),
        "json_token_share": _category_share(report, "detected_content", "json"),
        "unknown_token_share": _category_share(report, "detected_content", "unknown"),
        "tool_result_token_share": _category_share(report, "by_context_block_kind", "tool_result"),
        "tool_definition_token_share": _category_share(report, "by_context_block_kind", "tool_definition"),
        "exact_repetition_token_share": _number(repetition.get("exact_repeated_token_share")),
        "semantic_repetition_token_share": _number(repetition.get("semantic_repeated_token_share")),
        "cached_input_ratio": _number(usage.get("cache_ratio")),
        "stable_prefix_share": _number(stable_prefix.get("share")),
        "estimated_block_size_p50": _number(distributions.get("p50")),
        "estimated_block_size_p90": _number(distributions.get("p90")),
        "analysis_coverage": _number(quality.get("analysis_coverage")),
        "correlation_coverage": _number(quality.get("correlation_coverage")),
        "forwarding_errors": _number(quality.get("forwarding_errors")),
        "context_malformed": _number(quality.get("context_malformed")),
    }


def _share_stable(current: float | None, previous: float | None) -> bool | None:
    if current is None or previous is None:
        return None
    return abs(current - previous) < SHARE_TOLERANCE


def _distribution_stable(current: float | None, previous: float | None) -> bool | None:
    if current is None or previous is None:
        return None
    if current == previous:
        return True
    if current == 0 or previous == 0:
        return False
    return abs(current - previous) / max(abs(previous), abs(current)) < RELATIVE_TOLERANCE


def _top_three(report: dict[str, Any]) -> list[str]:
    return [str(row.get("name")) for row in report.get("opportunity_ranking", [])[:3]]


def _transition(current: dict[str, Any], previous: dict[str, Any]) -> dict[str, Any]:
    current_values = _metric_values(current)
    previous_values = _metric_values(previous)
    checks: dict[str, dict[str, Any]] = {}
    for name in (
        "plain_text_token_share",
        "json_token_share",
        "unknown_token_share",
        "tool_result_token_share",
        "tool_definition_token_share",
        "exact_repetition_token_share",
        "semantic_repetition_token_share",
        "cached_input_ratio",
        "stable_prefix_share",
    ):
        current_value = current_values[name]
        previous_value = previous_values[name]
        checks[name] = {
            "current": current_value,
            "previous": previous_value,
            "delta": None if current_value is None or previous_value is None else current_value - previous_value,
            "stable": _share_stable(current_value, previous_value),
            "tolerance": SHARE_TOLERANCE,
        }
    for name in ("estimated_block_size_p50", "estimated_block_size_p90"):
        current_value = current_values[name]
        previous_value = previous_values[name]
        checks[name] = {
            "current": current_value,
            "previous": previous_value,
            "relative_delta": (
                None
                if current_value is None or previous_value is None or previous_value == 0
                else (current_value - previous_value) / previous_value
            ),
            "stable": _distribution_stable(current_value, previous_value),
            "tolerance": RELATIVE_TOLERANCE,
        }
    checks["candidate_ranking_top3"] = {
        "current": _top_three(current),
        "previous": _top_three(previous),
        "stable": _top_three(current) == _top_three(previous),
    }
    stable_checks = [check["stable"] for check in checks.values()]
    return {
        "current_sessions": current.get("dataset", {}).get("sessions_total"),
        "previous_sessions": previous.get("dataset", {}).get("sessions_total"),
        "checks": checks,
        "stable": all(value is not False for value in stable_checks),
    }


def _quality_gate(report: dict[str, Any]) -> dict[str, Any]:
    values = _metric_values(report)
    checks = {
        name: {
            "value": values[name],
            "target": target,
            "pass": values[name] is not None and values[name] >= target,
        }
        for name, target in QUALITY_TARGETS.items()
    }
    checks["forwarding_errors"] = {
        "value": values["forwarding_errors"],
        "target": 0,
        "pass": values["forwarding_errors"] == 0,
    }
    checks["context_malformed"] = {
        "value": values["context_malformed"],
        "target": 0,
        "pass": values["context_malformed"] == 0,
    }
    return {"pass": all(check["pass"] for check in checks.values()), "checks": checks}


def evaluate_reports(reports: list[dict[str, Any]]) -> dict[str, Any]:
    """Return a deterministic convergence decision for reports ordered by cohort size."""

    reports = sorted(reports, key=lambda report: int(report.get("dataset", {}).get("sessions_total", 0)))
    sessions = int(reports[-1].get("dataset", {}).get("sessions_total", 0)) if reports else 0
    quality = [_quality_gate(report) for report in reports]
    transitions = [
        _transition(reports[index], reports[index - 1])
        for index in range(1, len(reports))
    ]
    reasons: list[str] = []
    if not reports:
        reasons.append("no reports supplied")
    if sessions < MIN_SESSIONS:
        reasons.append(f"minimum cohort size is {MIN_SESSIONS}; current is {sessions}")
    if len(transitions) < 2:
        reasons.append("two consecutive stable transitions are required")
    if any(not gate["pass"] for gate in quality):
        reasons.append("one or more measurement quality gates failed")
    if transitions and not transitions[-1]["stable"]:
        reasons.append("latest batch transition is not stable")
    if len(transitions) >= 2 and not all(transition["stable"] for transition in transitions[-2:]):
        reasons.append("the last two batch transitions are not both stable")
    converged = (
        bool(reports)
        and sessions >= MIN_SESSIONS
        and len(transitions) >= 2
        and all(gate["pass"] for gate in quality)
        and all(transition["stable"] for transition in transitions[-2:])
    )
    return {
        "status": "CONVERGED" if converged else "NEED_MORE_SESSIONS",
        "current_sessions": sessions,
        "minimum_sessions": MIN_SESSIONS,
        "share_tolerance": SHARE_TOLERANCE,
        "distribution_relative_tolerance": RELATIVE_TOLERANCE,
        "quality": quality,
        "transitions": transitions,
        "reasons": reasons or ["all convergence gates passed"],
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path, help="JSON reports ordered or unordered by cohort size")
    parser.add_argument("--json-output", type=Path)
    return parser


def main(arguments: list[str] | None = None) -> int:
    options = build_parser().parse_args(arguments)
    reports = []
    for path in options.reports:
        with path.open(encoding="utf-8") as handle:
            reports.append(json.load(handle))
    result = evaluate_reports(reports)
    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if options.json_output:
        options.json_output.parent.mkdir(parents=True, exist_ok=True)
        options.json_output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if result["status"] == "CONVERGED" else 1


if __name__ == "__main__":
    raise SystemExit(main())
