#!/usr/bin/env python3
"""Metadata-only offline ToolResult lifetime analysis for Phase 4.6.

This program reads persisted fingerprints, positions, and request ordinals from one or more
Tracepress SQLite databases.  It never selects raw request payloads or ToolResult content, and
does not modify the database.  Published aggregate reports are deliberately insufficient input:
lineage needs per-occurrence association metadata.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import UTC, datetime
import json
import math
from pathlib import Path
import sqlite3
from typing import Any, Iterable


REPORT_VERSION = 1
AGE_THRESHOLDS = (1, 2, 3, 5, 10)
SIZE_BUCKETS = (("<256", 0, 256), ("256-1K", 256, 1024), ("1K-4K", 1024, 4096),
                ("4K-16K", 4096, 16384), (">16K", 16384, None))
STUB_TEMPLATE = "[Historical tool result omitted by Tracepress. Recovery available: Rxxxxxxxxxxxxxxxxxxxxxxxx.]"
RECOVERY_SCHEMA = json.dumps({
    "type": "function", "name": "tracepress.recover",
    "description": "Recover a bounded range of a historical tool result by opaque recovery id.",
    "parameters": {"type": "object", "additionalProperties": False,
                   "required": ["recovery_id"],
                   "properties": {"recovery_id": {"type": "string"},
                                  "offset": {"type": "integer", "minimum": 0},
                                  "limit": {"type": "integer", "minimum": 1}}},
}, separators=(",", ":"))
POLICIES = (
    ("E0", "keep everything", lambda age, tokens: False),
    ("E1", "age >= 1", lambda age, tokens: age >= 1),
    ("E2", "age >= 2", lambda age, tokens: age >= 2),
    ("E3", "age >= 3", lambda age, tokens: age >= 3),
    ("E4", "age >= 2 and size >= 512 estimated tokens", lambda age, tokens: age >= 2 and tokens >= 512),
    ("E5", "age >= 2 and size >= 2048 estimated tokens", lambda age, tokens: age >= 2 and tokens >= 2048),
)


def percentile(values: Iterable[int | float], value: int) -> float | None:
    numbers = sorted(float(item) for item in values)
    if not numbers:
        return None
    offset = (len(numbers) - 1) * value / 100
    low, high = math.floor(offset), math.ceil(offset)
    return numbers[low] if low == high else numbers[low] + (numbers[high] - numbers[low]) * (offset - low)


def distribution(values: Iterable[int | float]) -> dict[str, Any]:
    numbers = list(values)
    return {"count": len(numbers), **{f"p{point}": percentile(numbers, point) for point in (50, 75, 90, 95, 99)},
            "max": max(numbers) if numbers else None}


def ratio(numerator: int | float, denominator: int | float) -> float | None:
    return None if not denominator else numerator / denominator


def estimated_tokens_from_bytes(raw_bytes: int) -> int:
    """Match the report's explicit heuristic declaration; it is not provider usage."""
    return max(1, math.ceil(raw_bytes / 4))


def fingerprint(value: Any) -> str | None:
    if value is None:
        return None
    if isinstance(value, memoryview):
        value = value.tobytes()
    if isinstance(value, bytes):
        return value.hex()
    return str(value)


def readonly_connection(path: Path) -> sqlite3.Connection:
    connection = sqlite3.connect(f"file:{path.resolve()}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA query_only = ON")
    return connection


def columns(connection: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in connection.execute(f"PRAGMA table_info({table})")}


def require_schema(connection: sqlite3.Connection) -> None:
    needed = {
        "context_snapshots": {"snapshot_id", "session_id", "inference_operation_id"},
        "context_block_occurrences": {"snapshot_id", "kind", "exact_fingerprint", "raw_bytes", "estimated_tokens"},
    }
    for table, required in needed.items():
        missing = required - columns(connection, table)
        if missing:
            raise ValueError(f"{table} lacks required Phase 4.6 metadata: {', '.join(sorted(missing))}")


def load_workloads(path: Path | None) -> dict[str, str]:
    if path is None:
        return {}
    value = json.loads(path.read_text(encoding="utf-8"))
    ledger = value.get("request_analysis_ledger", []) if isinstance(value, dict) else []
    return {str(row["snapshot_id"]): str(row["workload"])
            for row in ledger if row.get("snapshot_id") and row.get("workload")}


def load_occurrences(path: Path, workloads: dict[str, str], default_workload: str) -> tuple[list[dict[str, Any]], list[str]]:
    connection = readonly_connection(path)
    try:
        require_schema(connection)
        snapshot_columns = columns(connection, "context_snapshots")
        block_columns = columns(connection, "context_block_occurrences")
        request_columns = columns(connection, "provider_requests")
        completion = "s.completed_at_us" if "completed_at_us" in snapshot_columns else "NULL"
        started = "s.started_at_us" if "started_at_us" in snapshot_columns else "NULL"
        request_kind = "p.request_kind" if "request_kind" in request_columns else "NULL"
        join_request = "LEFT JOIN provider_requests p ON p.request_id = s.provider_request_id" if request_columns else ""
        tool_call = "b.tool_call_id" if "tool_call_id" in block_columns else "NULL"
        tool_name = "b.tool_name" if "tool_name" in block_columns else "NULL"
        semantic = "b.semantic_fingerprint" if "semantic_fingerprint" in block_columns else "NULL"
        raw_start = "b.raw_value_start" if "raw_value_start" in block_columns else "NULL"
        query = f"""
          SELECT s.snapshot_id, s.session_id, s.inference_operation_id, {completion} AS completed_at_us,
                 {started} AS started_at_us, {request_kind} AS request_kind,
                 b.block_occurrence_id, b.ordinal, b.kind, b.raw_bytes, b.estimated_tokens,
                 b.exact_fingerprint, {semantic} AS semantic_fingerprint, {tool_call} AS tool_call_id,
                 {tool_name} AS tool_name, b.detected_kind, {raw_start} AS raw_value_start
          FROM context_snapshots s {join_request}
          JOIN context_block_occurrences b ON b.snapshot_id = s.snapshot_id
          WHERE s.status = 'complete'
        """
        rows = [dict(row) for row in connection.execute(query)]
    finally:
        connection.close()

    snapshots: dict[tuple[str, str], int] = {}
    by_session: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        by_session[str(row["session_id"])].append(row)
    for session, items in by_session.items():
        ordered = sorted(items, key=lambda row: (row["completed_at_us"] is None, row["completed_at_us"] or row["started_at_us"] or 0, row["snapshot_id"]))
        for ordinal, snapshot_id in enumerate(dict.fromkeys(str(row["snapshot_id"]) for row in ordered), 1):
            snapshots[(session, snapshot_id)] = ordinal

    output = []
    for row in rows:
        row["database"] = path.name
        row["request_ordinal"] = snapshots[(str(row["session_id"]), str(row["snapshot_id"]))]
        row["workload"] = workloads.get(str(row["snapshot_id"]), default_workload)
        row["exact_fingerprint"] = fingerprint(row["exact_fingerprint"])
        row["semantic_fingerprint"] = fingerprint(row["semantic_fingerprint"])
        row["estimated_tokens"] = row["estimated_tokens"] if row["estimated_tokens"] is not None else estimated_tokens_from_bytes(int(row["raw_bytes"]))
        output.append(row)
    return output, sorted({str(row["snapshot_id"]) for row in rows})


def size_bucket(tokens: int) -> str:
    for name, low, high in SIZE_BUCKETS:
        if tokens >= low and (high is None or tokens < high):
            return name
    raise AssertionError("unreachable")


def risk(prefix_ratio: float | None) -> str:
    if prefix_ratio is None:
        return "Unknown"
    if prefix_ratio >= .90:
        return "Low"
    if prefix_ratio >= .50:
        return "Medium"
    return "High"


def build_lineages(rows: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], dict[str, int]]:
    """Fail closed when call association or an exact fingerprint is absent.

    Exact content alone is never an identity: a new call with equal output begins a separate
    lineage. A later occurrence must carry the same session and call association to join it.
    """
    tool_rows = [row for row in rows if row["kind"] == "tool_result"]
    ordered = sorted(tool_rows, key=lambda row: (row["database"], row["session_id"], row["request_ordinal"], row["ordinal"], row["block_occurrence_id"]))
    active: dict[tuple[str, str, str, str], dict[str, Any]] = {}
    semantic_index: dict[tuple[str, str, str], list[dict[str, Any]]] = defaultdict(list)
    exclusions = Counter()
    for row in ordered:
        if not row["tool_call_id"]:
            exclusions["missing_tool_call_id"] += 1
            continue
        if not row["exact_fingerprint"]:
            exclusions["missing_exact_fingerprint"] += 1
            continue
        key = (row["database"], row["session_id"], str(row["tool_call_id"]), row["exact_fingerprint"])
        lineage = active.get(key)
        if lineage is None:
            lineage = {"lineage_id": f"L{len(active) + 1:06d}", "database": row["database"], "session_id": row["session_id"],
                       "producer_operation": row["inference_operation_id"], "tool_call_id": row["tool_call_id"],
                       "first_seen_request": row["request_ordinal"], "occurrences": [], "semantic_reappearances": 0}
            active[key] = lineage
            if row["semantic_fingerprint"]:
                semantic_index[(row["database"], row["session_id"], row["semantic_fingerprint"])].append(lineage)
        elif row["request_ordinal"] > lineage["first_seen_request"]:
            lineage["exact_reappearances"] = lineage.get("exact_reappearances", 0) + 1
        if row["semantic_fingerprint"]:
            for related in semantic_index[(row["database"], row["session_id"], row["semantic_fingerprint"])]:
                if related is not lineage and row["request_ordinal"] > related["first_seen_request"]:
                    related["semantic_reappearances"] += 1
        lineage["occurrences"].append(row)
    for lineage in active.values():
        occurrences = lineage["occurrences"]
        lineage["last_seen_request"] = max(row["request_ordinal"] for row in occurrences)
        lineage["requests_present"] = len({row["request_ordinal"] for row in occurrences})
        lineage["requests_after_creation"] = lineage["last_seen_request"] - lineage["first_seen_request"]
        first = occurrences[0]
        lineage.update({"estimated_tokens": first["estimated_tokens"], "raw_bytes": first["raw_bytes"],
                        "content_kind": first["detected_kind"] or "unknown", "tool_family": first["tool_name"] or "unknown",
                        "workload": first["workload"]})
        lineage["cumulative_token_exposure"] = sum(row["estimated_tokens"] for row in occurrences)
        lineage.setdefault("exact_reappearances", 0)
    return list(active.values()), dict(exclusions)


def report(rows: list[dict[str, Any]], snapshot_count: int, source_paths: list[Path]) -> dict[str, Any]:
    lineages, exclusions = build_lineages(rows)
    total_context = sum(int(row["estimated_tokens"]) for row in rows)
    session_last_request = {
        (row["database"], row["session_id"]): max(
            item["request_ordinal"] for item in rows
            if item["database"] == row["database"] and item["session_id"] == row["session_id"]
        )
        for row in rows
    }
    lineages_with_follow_up = sum(
        session_last_request[(lineage["database"], lineage["session_id"])] > lineage["first_seen_request"]
        for lineage in lineages
    )
    historical = [(lineage, occurrence) for lineage in lineages for occurrence in lineage["occurrences"]
                  if occurrence["request_ordinal"] > lineage["first_seen_request"]]
    historical_exposure = sum(item[1]["estimated_tokens"] for item in historical)
    age_buckets = []
    for threshold in AGE_THRESHOLDS:
        exposure = sum(occurrence["estimated_tokens"] for lineage, occurrence in historical
                       if occurrence["request_ordinal"] - lineage["first_seen_request"] >= threshold)
        age_buckets.append({"age_at_least": threshold, "estimated_exposure": exposure,
                            "share_of_total_context": ratio(exposure, total_context)})
    matrix = []
    for size, _, _ in SIZE_BUCKETS:
        for label, threshold in [("1", 1), ("2", 2), ("3", 3), ("5", 5), ("10+", 10)]:
            values = [(lineage, occurrence) for lineage, occurrence in historical if size_bucket(lineage["estimated_tokens"]) == size and occurrence["request_ordinal"] - lineage["first_seen_request"] >= threshold]
            exposure = sum(occurrence["estimated_tokens"] for _, occurrence in values)
            matrix.append({"size": size, "age": label, "toolresults": len(values), "estimated_exposure": exposure,
                           "share_of_total_context": ratio(exposure, total_context)})
    stub_bytes, stub_tokens = len(STUB_TEMPLATE.encode()), estimated_tokens_from_bytes(len(STUB_TEMPLATE.encode()))
    schema_bytes, schema_tokens = len(RECOVERY_SCHEMA.encode()), estimated_tokens_from_bytes(len(RECOVERY_SCHEMA.encode()))
    simulations = []
    for policy_id, description, eligible in POLICIES:
        selected = [(lineage, occurrence) for lineage, occurrence in historical
                    if eligible(occurrence["request_ordinal"] - lineage["first_seen_request"], lineage["estimated_tokens"])]
        gross = sum(occurrence["estimated_tokens"] for _, occurrence in selected)
        stubs = len(selected) * stub_tokens
        affected_sessions = {lineage["session_id"] for lineage, _ in selected}
        # A recovery tool schema is present once per provider request, never once per context
        # block. Counting block rows here would overstate its context cost.
        schema_occurrences = len({row["snapshot_id"] for row in rows if row["session_id"] in affected_sessions})
        schema_overhead = schema_occurrences * schema_tokens
        net = gross - stubs - schema_overhead
        risks = Counter()
        for lineage, occurrence in selected:
            total_bytes = sum(row["raw_bytes"] for row in rows if row["snapshot_id"] == occurrence["snapshot_id"])
            offset = occurrence["raw_value_start"]
            prefix_ratio = ratio(offset, total_bytes) if offset is not None else None
            risks[risk(prefix_ratio)] += 1
        simulations.append({"policy_id": policy_id, "description": description, "eligible_historical_occurrences": len(selected),
                            "gross_eviction_reduction": gross, "stub_overhead": stubs, "recovery_tool_schema_overhead": schema_overhead,
                            "shadow_net_eviction_reduction": net, "shadow_net_total_context_reduction": ratio(net, total_context),
                            "cache_risk_distribution": dict(sorted(risks.items()))})
    grouped = defaultdict(list)
    for lineage in lineages:
        for key in ((lineage["content_kind"], lineage["tool_family"], "all"),
                    (lineage["content_kind"], "all", "all"),
                    ("all", lineage["tool_family"], "all"),
                    ("all", "all", lineage["workload"])):
            grouped[key].append(lineage)
    distributions = [{"content_kind": key[0], "tool_family": key[1], "workload": key[2],
                      "lifetime": distribution(item["requests_after_creation"] for item in values), "lineages": len(values)}
                     for key, values in sorted(grouped.items())]
    compaction_snapshot_ids = {row["snapshot_id"] for row in rows if row["request_kind"] == "compaction"}
    compaction = {"available": bool(compaction_snapshot_ids),
                  "compaction_snapshots": len(compaction_snapshot_ids),
                  "note": "A compaction boundary is a measured request-kind marker, not an assumed safe cache reset."}
    best = max(simulations, key=lambda item: item["shadow_net_total_context_reduction"] or float("-inf"))
    gate = {"historical_exposure_threshold": .10, "shadow_net_total_context_reduction_threshold": .05,
            "lineages_with_follow_up_observation": lineages_with_follow_up,
            "historical_toolresult_share": ratio(historical_exposure, total_context),
            "best_policy": best["policy_id"], "best_shadow_net_total_context_reduction": best["shadow_net_total_context_reduction"],
            "passed": lineages_with_follow_up > 0 and ((ratio(historical_exposure, total_context) or 0) >= .10 or (best["shadow_net_total_context_reduction"] or 0) >= .05),
            "decision": (
                "insufficient_follow_up_observation"
                if lineages_with_follow_up == 0
                else "shadow_eviction_may_be_investigated"
                if ((ratio(historical_exposure, total_context) or 0) >= .10 or (best["shadow_net_total_context_reduction"] or 0) >= .05)
                else "close_phase_4_6_without_runtime_eviction"
            )}
    return {"report_version": REPORT_VERSION, "generated_at_utc": datetime.now(UTC).isoformat(), "phase": "4.6_offline", "sources": [path.name for path in source_paths],
            "privacy": {"metadata_only": True, "raw_content_persisted": False}, "dataset": {"snapshots": snapshot_count, "blocks": len(rows), "estimated_total_context": total_context, "lineage_coverage_exclusions": exclusions},
            "lineages": {"count": len(lineages), "lifetime_distributions": distributions,
                         "retained_token_exposure": historical_exposure, "historical_toolresult_share": ratio(historical_exposure, total_context)},
            "age_buckets": age_buckets, "size_age_matrix": matrix,
            "recovery_stub": {"template": "opaque recovery reference", "stub_bytes": stub_bytes, "stub_estimated_tokens": stub_tokens,
                              "recovery_tool_schema_bytes": schema_bytes, "recovery_tool_schema_estimated_tokens": schema_tokens},
            "policy_simulations": simulations, "cache_risk_proxy": {"classification": {"Low": "preserved prefix >= 90%", "Medium": "50%-<90%", "High": "<50%", "Unknown": "no raw offset"}, "structural_only": True},
            "compaction_interaction": compaction, "offline_gate": gate,
            "limitations": ["Local token estimates are heuristic, not provider usage.", "Cache risk is a structural prefix proxy, not provider cache behavior.", "Lineages without call association or exact fingerprints fail closed and are excluded.", "No forwarding request was read or modified."]}


def markdown(value: dict[str, Any]) -> str:
    gate = value["offline_gate"]
    lines = ["# TRACEPRESS_TOOLRESULT_LIFETIME_001", "", "## Scope", "", "Offline, metadata-only ToolResult lifetime analysis. No forwarding mutation, recovery object, or raw ToolResult content is produced.", "", "## Dataset", "", f"- Snapshots: {value['dataset']['snapshots']}", f"- Context blocks: {value['dataset']['blocks']}", f"- ToolResult lineages: {value['lineages']['count']}", f"- Estimated total context: {value['dataset']['estimated_total_context']}", "", "## Historical exposure", "", f"- Retained token exposure: {value['lineages']['retained_token_exposure']}", f"- Historical ToolResult share: {value['lineages']['historical_toolresult_share']}", "", "## Offline gate", "", f"- Lineages with a later request available: {gate['lineages_with_follow_up_observation']}", f"- Best policy: `{gate['best_policy']}`", f"- Best shadow net total-context reduction: {gate['best_shadow_net_total_context_reduction']}", f"- Decision: `{gate['decision']}`", "", "## Limitations", ""]
    lines.extend(f"- {item}" for item in value["limitations"])
    lines.extend(["", "## Lifetime distributions", "", "| Content kind | Tool family | Workload | Lineages | P50 | P95 | P99 | Max |", "|---|---|---|---:|---:|---:|---:|---:|"])
    for item in value["lineages"]["lifetime_distributions"]:
        lifetime = item["lifetime"]
        lines.append(f"| `{item['content_kind']}` | `{item['tool_family']}` | `{item['workload']}` | {item['lineages']} | {lifetime['p50']} | {lifetime['p95']} | {lifetime['p99']} | {lifetime['max']} |")
    lines.extend(["", "## Age buckets", "", "| Age | Estimated exposure | Share of total context |", "|---:|---:|---:|"])
    lines.extend(f"| >= {item['age_at_least']} | {item['estimated_exposure']} | {item['share_of_total_context']} |" for item in value["age_buckets"])
    lines.extend(["", "## Size x age", "", "| Size | Age | ToolResults | Estimated exposure | Share of total context |", "|---|---:|---:|---:|---:|"])
    lines.extend(f"| `{item['size']}` | {item['age']} | {item['toolresults']} | {item['estimated_exposure']} | {item['share_of_total_context']} |" for item in value["size_age_matrix"])
    stub = value["recovery_stub"]
    lines.extend(["", "## Stub and recovery schema overhead", "", f"- Stub: {stub['stub_bytes']} bytes; {stub['stub_estimated_tokens']} locally estimated tokens.", f"- Recovery schema: {stub['recovery_tool_schema_bytes']} bytes; {stub['recovery_tool_schema_estimated_tokens']} locally estimated tokens.", "", "## Policy simulations", "", "| Policy | Eligible historical ToolResults | Gross reduction | Stub overhead | Recovery schema overhead | Shadow net reduction | Net total-context reduction | Cache-risk distribution |", "|---|---:|---:|---:|---:|---:|---:|---|"])
    for item in value["policy_simulations"]:
        lines.append(f"| `{item['policy_id']}` | {item['eligible_historical_occurrences']} | {item['gross_eviction_reduction']} | {item['stub_overhead']} | {item['recovery_tool_schema_overhead']} | {item['shadow_net_eviction_reduction']} | {item['shadow_net_total_context_reduction']} | `{item['cache_risk_distribution']}` |")
    compaction = value["compaction_interaction"]
    lines.extend(["", "## Cache-risk proxy and compaction", "", f"- Cache proxy is structural only: `{value['cache_risk_proxy']['classification']}`.", f"- Compaction snapshots observed: {compaction['compaction_snapshots']}; available: {str(compaction['available']).lower()}.", f"- {compaction['note']}"])
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True, action="append", type=Path, help="read-only Tracepress SQLite source; repeat for cohorts")
    parser.add_argument("--workload-report", type=Path, help="Baseline report carrying the safe snapshot-to-workload ledger")
    parser.add_argument("--default-workload", default="unmapped", help="safe cohort label when no snapshot ledger is supplied")
    parser.add_argument("--output-json", required=True, type=Path)
    parser.add_argument("--output-md", required=True, type=Path)
    args = parser.parse_args()
    workloads = load_workloads(args.workload_report)
    rows, snapshots = [], set()
    for database in args.database:
        loaded, ids = load_occurrences(database, workloads, args.default_workload)
        rows.extend(loaded)
        snapshots.update(ids)
    value = report(rows, len(snapshots), args.database)
    args.output_json.parent.mkdir(parents=True, exist_ok=True)
    args.output_json.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.output_md.write_text(markdown(value), encoding="utf-8")


if __name__ == "__main__":
    main()
