#!/usr/bin/env python3
"""Generate metadata-only Shadow Compression experiment reports from SQLite."""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
import math
from pathlib import Path
import sqlite3
from typing import Any, Iterable


REQUIRED_GATES = (
    "shadow_pilot_n10",
    "compaction_calibration",
    "aa_infrastructure",
    "host_rss_cpu_comparison",
    "naturalistic_workload_distributions",
)
CONTROL_COMPRESSORS = {"json.noop", "text.noop"}


def percentile(values: Iterable[int | float], quantile: float) -> float | None:
    ordered = sorted(float(value) for value in values)
    if not ordered:
        return None
    position = (len(ordered) - 1) * quantile
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def ratio(numerator: int, denominator: int) -> float | None:
    return None if denominator == 0 else numerator / denominator


def load_manifest(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {}
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("manifest must be a JSON object")
    return value


def readonly_connection(path: Path) -> sqlite3.Connection:
    connection = sqlite3.connect(f"file:{path.resolve()}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA query_only = ON")
    return connection


def experiment_row(connection: sqlite3.Connection, experiment_id: str) -> dict[str, Any]:
    row = connection.execute(
        """SELECT experiment_id, compressor_set_json, runtime_sha, limits_json, status,
                  started_at, completed_at, forwarding_mutations, shadow_drops,
                  recovery_failures, determinism_failures
             FROM compression_experiments WHERE experiment_id = ?""",
        (experiment_id,),
    ).fetchone()
    if row is None:
        raise ValueError(f"compression experiment not found: {experiment_id}")
    result = dict(row)
    result["compressor_set"] = json.loads(result.pop("compressor_set_json"))
    result["limits"] = json.loads(result.pop("limits_json"))
    return result


def candidate_rows(connection: sqlite3.Connection, experiment_id: str) -> list[dict[str, Any]]:
    columns = {row[1] for row in connection.execute("PRAGMA table_info(compression_candidates)")}
    provider_readability = "c.provider_readability" if "provider_readability" in columns else "'unknown'"
    json_root_kind = "c.json_root_kind" if "json_root_kind" in columns else "NULL"
    text_shape = "c.text_shape" if "text_shape" in columns else "NULL"
    rows = connection.execute(
        f"""SELECT c.compressor_id, c.compressor_version, c.status,
                  c.original_fingerprint, c.cache_risk, m.input_bytes, m.output_bytes,
                  m.bytes_delta, m.input_estimated_tokens, m.output_estimated_tokens,
                  m.estimated_token_delta, m.processing_us, m.reversible,
                  m.recovery_verified, m.deterministic,
                  m.preserved_prefix_ratio_basis_points, cs.session_id,
                  {provider_readability} AS provider_readability,
                  {json_root_kind} AS json_root_kind, {text_shape} AS text_shape
             FROM compression_candidates c
             JOIN compression_candidate_metrics m USING(candidate_id)
             JOIN context_snapshots cs USING(snapshot_id)
            WHERE c.experiment_id = ?""",
        (experiment_id,),
    ).fetchall()
    return [dict(row) for row in rows]


def sum_present(rows: Iterable[dict[str, Any]], key: str) -> int | None:
    values = [int(row[key]) for row in rows if row.get(key) is not None]
    return sum(values) if values else None


def aggregate_compressors(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[(row["compressor_id"], row["compressor_version"])].append(row)

    result = []
    for (compressor_id, version), group in sorted(grouped.items()):
        applicable = [row for row in group if row["status"] == "applicable"]
        processing_times = [
            row["processing_us"] for row in group if row["processing_us"] is not None
        ]
        recovery_attempted = [row for row in group if row["reversible"] and row["output_bytes"] is not None]
        determinism_checked = [row for row in group if row["output_bytes"] is not None]
        unique: dict[bytes, dict[str, Any]] = {}
        for row in applicable:
            unique.setdefault(bytes(row["original_fingerprint"]), row)
        input_bytes = sum_present(applicable, "input_bytes")
        byte_reduction = sum_present(applicable, "bytes_delta")
        estimated_input = sum_present(applicable, "input_estimated_tokens")
        estimated_reduction = sum_present(applicable, "estimated_token_delta")
        eligible_estimated = sum_present(group, "input_estimated_tokens")
        status_counts = Counter(row["status"] for row in group)
        result.append(
            {
                "id": compressor_id,
                "version": version,
                "control": compressor_id in CONTROL_COMPRESSORS,
                "eligible_blocks": len(group),
                "applicable_blocks": len(applicable),
                "applicability": ratio(len(applicable), len(group)),
                "addressable_token_share": ratio(estimated_input or 0, eligible_estimated or 0),
                "provider_readability": sorted({row.get("provider_readability") or "unknown" for row in group}),
                "shape_distribution": dict(sorted(Counter(
                    row.get("json_root_kind") or row.get("text_shape") or "unavailable"
                    for row in group
                ).items())),
                "status_counts": dict(sorted(status_counts.items())),
                "input_bytes": input_bytes,
                "output_bytes": sum_present(applicable, "output_bytes"),
                "byte_reduction": byte_reduction,
                "byte_reduction_ratio": (
                    ratio(byte_reduction, input_bytes)
                    if byte_reduction is not None and input_bytes is not None
                    else None
                ),
                "median_byte_reduction_ratio": percentile(
                    (
                        row["bytes_delta"] / row["input_bytes"]
                        for row in applicable
                        if row["bytes_delta"] is not None and row["input_bytes"]
                    ),
                    0.50,
                ),
                "estimated_input_tokens": estimated_input,
                "estimated_output_tokens": sum_present(applicable, "output_estimated_tokens"),
                "estimated_token_reduction": estimated_reduction,
                "estimated_token_reduction_ratio": (
                    ratio(estimated_reduction, estimated_input)
                    if estimated_reduction is not None and estimated_input is not None
                    else None
                ),
                "median_estimated_token_reduction_ratio": percentile(
                    (
                        row["estimated_token_delta"] / row["input_estimated_tokens"]
                        for row in applicable
                        if row["estimated_token_delta"] is not None and row["input_estimated_tokens"]
                    ),
                    0.50,
                ),
                "candidate_effective_byte_reduction": byte_reduction,
                "unique_candidate_byte_reduction": sum_present(unique.values(), "bytes_delta"),
                "candidate_effective_estimated_token_reduction": estimated_reduction,
                "unique_candidate_estimated_token_reduction": sum_present(
                    unique.values(), "estimated_token_delta"
                ),
                "processing_us": {
                    "p50": percentile(processing_times, 0.50),
                    "p90": percentile(processing_times, 0.90),
                    "p95": percentile(processing_times, 0.95),
                    "p99": percentile(processing_times, 0.99),
                },
                "recovery_success_rate": ratio(
                    sum(bool(row["recovery_verified"]) for row in recovery_attempted),
                    len(recovery_attempted),
                ),
                "determinism_success_rate": ratio(
                    sum(bool(row["deterministic"]) for row in determinism_checked),
                    len(determinism_checked),
                ),
                "preserved_prefix_ratio": {
                    "p50": percentile(
                        (
                            row["preserved_prefix_ratio_basis_points"] / 10_000
                            for row in applicable
                            if row["preserved_prefix_ratio_basis_points"] is not None
                        ),
                        0.50,
                    )
                },
                "cache_risk": dict(sorted(Counter(row["cache_risk"] for row in group).items())),
            }
        )
    return result


def workload_distribution(
    rows: list[dict[str, Any]], session_workloads: dict[str, Any]
) -> dict[str, Any]:
    if not session_workloads:
        return {"available": False, "reason": "session workload manifest was not supplied"}
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        workload = str(session_workloads.get(row["session_id"], "unmapped"))
        grouped[workload].append(row)
    workloads = []
    for workload, workload_rows in sorted(grouped.items()):
        workloads.append(
            {
                "workload": workload,
                "session_count": len({row["session_id"] for row in workload_rows}),
                "candidate_count": len(workload_rows),
                "compressors": aggregate_compressors(workload_rows),
            }
        )
    return {"available": True, "workloads": workloads}


def select_candidate(
    experiment: dict[str, Any],
    compressors: list[dict[str, Any]],
    gates: dict[str, str],
    requested_candidate: Any,
) -> tuple[str | None, str]:
    if experiment["forwarding_mutations"] != 0:
        return None, "blocked_forwarding_mutations_detected"
    if any(gates.get(gate) != "passed" for gate in REQUIRED_GATES):
        return None, "blocked_pending_empirical_gates"
    if not isinstance(requested_candidate, str) or not requested_candidate:
        return None, "blocked_candidate_decision_not_recorded"
    eligible = [
        item
        for item in compressors
        if not item["control"]
        and (
            item["provider_readability"] == ["human_readable_structured"]
            # Legacy pre-0011 databases have no readability column. Preserve the old
            # diagnostic selection behavior for those reports; every current database
            # records explicit readability and therefore cannot take this branch.
            or (item["id"] == "json.minify" and item["provider_readability"] == ["unknown"])
        )
        and item["applicable_blocks"] > 0
        and item["byte_reduction_ratio"] is not None
        and item["byte_reduction_ratio"] > 0
        and item["recovery_success_rate"] == 1.0
        and item["determinism_success_rate"] == 1.0
    ]
    if not any(item["id"] == requested_candidate for item in eligible):
        return None, "blocked_requested_candidate_did_not_qualify"
    return requested_candidate, "selected_after_all_gates_passed"


def build_report(
    connection: sqlite3.Connection, experiment_id: str, manifest: dict[str, Any]
) -> dict[str, Any]:
    experiment = experiment_row(connection, experiment_id)
    rows = candidate_rows(connection, experiment_id)
    included_session_ids = manifest.get("included_session_ids")
    if included_session_ids is not None:
        if not isinstance(included_session_ids, list) or not all(
            isinstance(value, str) for value in included_session_ids
        ):
            raise ValueError("included_session_ids must be an array of strings")
        included = set(included_session_ids)
        rows = [row for row in rows if row["session_id"] in included]
    compressors = aggregate_compressors(rows)
    supplied_gates = manifest.get("quality_gates", {})
    gates = {gate: supplied_gates.get(gate, "pending") for gate in REQUIRED_GATES}
    recommended, recommendation_status = select_candidate(
        experiment,
        compressors,
        gates,
        manifest.get("recommended_active_candidate"),
    )
    sessions = {row["session_id"] for row in rows}
    return {
        "report_id": manifest.get("report_id", "TRACEPRESS_SHADOW_COMPRESSION_001"),
        "report_version": 1,
        "experiment": experiment,
        "scope": {
            "session_count": len(sessions),
            "candidate_count": len(rows),
            "unknown_transformed": 0,
            "included_session_ids": sorted(sessions),
            "excluded_session_ids": manifest.get("excluded_session_ids", []),
        },
        "compressors": compressors,
        "workload_distribution": workload_distribution(
            rows, manifest.get("session_workloads", {})
        ),
        "quality_gates": gates,
        "measurement_context": manifest.get("measurement_context", {}),
        "collection_notes": manifest.get("attempt_notes", []),
        "invariants": {
            "active_request_rewriting": False,
            "forwarding_mutations": experiment["forwarding_mutations"],
            "unknown_transform_policy": "never",
            "candidate_content_persisted": False,
        },
        "recommended_active_candidate": recommended,
        "recommendation_rationale": manifest.get("recommendation_rationale"),
        "recommendation_status": recommendation_status,
        "phase_4_2": "ready_for_design" if recommended else "blocked",
        "phase_4_2_1": "ready_for_active_ab" if recommended else "blocked",
        "limitations": [
            "Candidate reduction is local representation evidence, not provider token savings.",
            "Cache risk is structural evidence, not a provider cache prediction.",
            "This metadata-only report does not establish model quality preservation.",
        ],
    }


def format_percent(value: float | None) -> str:
    return "—" if value is None else f"{value * 100:.2f}%"


def format_microseconds(value: float | None) -> str:
    return "—" if value is None else f"{value:.1f} us"


def markdown(report: dict[str, Any]) -> str:
    experiment = report["experiment"]
    lines = [
        f"# {report['report_id'].replace('_', ' ').title()}",
        "",
        f"Experiment: `{experiment['experiment_id']}` · Status: **{experiment['status']}**",
        "",
        "This report measures shadow candidate reduction only. It does not claim provider token, cost, cache, or quality impact.",
        "",
        "## Integrity",
        "",
        f"- Forwarding mutations: {experiment['forwarding_mutations']}",
        f"- Shadow drops: {experiment['shadow_drops']}",
        f"- Recovery failures: {experiment['recovery_failures']}",
        f"- Determinism failures: {experiment['determinism_failures']}",
        "- Unknown transformed: 0",
        "",
        "## Candidate comparison",
        "",
        "| Candidate | Readability | Addressable | Applicable | Median byte ↓ | Byte reduction | Est. token reduction | Recovery | P95 |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for item in report["compressors"]:
        lines.append(
            f"| `{item['id']}` | {', '.join(item['provider_readability'])} | "
            f"{format_percent(item['addressable_token_share'])} | "
            f"{format_percent(item['applicability'])} | "
            f"{format_percent(item['median_byte_reduction_ratio'])} | "
            f"{format_percent(item['byte_reduction_ratio'])} | "
            f"{format_percent(item['estimated_token_reduction_ratio'])} | "
            f"{format_percent(item['recovery_success_rate'])} | "
            f"{format_microseconds(item['processing_us']['p95'])} |"
        )
    if report["collection_notes"]:
        lines.extend(["", "## Collection notes", ""])
        lines.extend(f"- {note}" for note in report["collection_notes"])
    lines.extend(["", "## Empirical gates", ""])
    for gate, status in report["quality_gates"].items():
        lines.append(f"- `{gate}`: **{status}**")
    lines.extend(
        [
            "",
            "## Recommendation",
            "",
            (
                f"Recommended active candidate: `{report['recommended_active_candidate']}`."
                if report["recommended_active_candidate"]
                else "No active candidate selected. Phase 4.2 remains blocked."
            ),
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--experiment-id", required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    args = parser.parse_args()
    manifest = load_manifest(args.manifest)
    with readonly_connection(args.db) as connection:
        report = build_report(connection, args.experiment_id, manifest)
    args.output_json.parent.mkdir(parents=True, exist_ok=True)
    args.output_md.parent.mkdir(parents=True, exist_ok=True)
    args.output_json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    args.output_md.write_text(markdown(report), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
