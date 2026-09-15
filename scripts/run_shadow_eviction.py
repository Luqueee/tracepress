#!/usr/bin/env python3
"""Emit bounded, metadata-only Phase 4.6 Shadow Eviction candidates.

The input database is opened read-only. This evaluator never reads raw ToolResult payloads and
never contacts a provider; forwarding remains byte-exact by construction.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
ANALYZER_PATH = ROOT / "scripts" / "analyze_toolresult_lifetime.py"
SPEC = importlib.util.spec_from_file_location("toolresult_lifetime", ANALYZER_PATH)
assert SPEC and SPEC.loader
ANALYZER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ANALYZER
SPEC.loader.exec_module(ANALYZER)

POLICIES = {item[0]: item for item in ANALYZER.POLICIES if item[0] in {"E1", "E2", "E5"}}
POLICY_VERSION = "v1"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--output-json", required=True, type=Path)
    parser.add_argument("--output-md", required=True, type=Path)
    parser.add_argument("--policy", action="append", choices=sorted(POLICIES), help="repeat; defaults to E1, E2, E5")
    parser.add_argument("--max-candidates", type=int, default=10_000)
    parser.add_argument("--default-workload", default="unmapped")
    return parser.parse_args()


def candidate_rows(rows: list[dict[str, Any]], selected_ids: list[str], limit: int) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    if limit <= 0:
        raise ValueError("max-candidates must be positive")
    lineages, exclusions = ANALYZER.build_lineages(rows)
    total_by_snapshot: dict[str, int] = {}
    for row in rows:
        total_by_snapshot[row["snapshot_id"]] = total_by_snapshot.get(row["snapshot_id"], 0) + int(row["raw_bytes"])
    stub_bytes = len(ANALYZER.STUB_TEMPLATE.encode())
    stub_tokens = ANALYZER.estimated_tokens_from_bytes(stub_bytes)
    schema_tokens = ANALYZER.estimated_tokens_from_bytes(len(ANALYZER.RECOVERY_SCHEMA.encode()))
    pending: list[tuple[str, dict[str, Any], dict[str, Any]]] = []
    for policy_id in selected_ids:
        _, _, applies = POLICIES[policy_id]
        for lineage in lineages:
            for occurrence in lineage["occurrences"]:
                age = occurrence["request_ordinal"] - lineage["first_seen_request"]
                if age > 0 and applies(age, lineage["estimated_tokens"]):
                    pending.append((policy_id, lineage, occurrence))
    pending.sort(key=lambda item: (item[0], item[1]["lineage_id"], item[2]["request_ordinal"], item[2]["ordinal"]))
    dropped = max(0, len(pending) - limit)
    pending = pending[:limit]
    per_policy_count = {policy_id: sum(item[0] == policy_id for item in pending) for policy_id in selected_ids}
    per_policy_sessions = {
        policy_id: {lineage["session_id"] for current, lineage, _ in pending if current == policy_id}
        for policy_id in selected_ids
    }
    schema_budget = {
        # The schema appears once in each affected provider request, rather than once for every
        # block parsed from that request.
        policy_id: schema_tokens * len({row["snapshot_id"] for row in rows if row["session_id"] in per_policy_sessions[policy_id]})
        for policy_id in selected_ids
    }
    emitted_per_policy = {policy_id: 0 for policy_id in selected_ids}
    candidates = []
    for policy_id, lineage, occurrence in pending:
        denominator = per_policy_count[policy_id]
        position = emitted_per_policy[policy_id]
        emitted_per_policy[policy_id] += 1
        # Deterministically allocate the session-wide recovery-schema burden across candidates.
        schema_share = schema_budget[policy_id] // denominator + int(position < schema_budget[policy_id] % denominator)
        total_bytes = total_by_snapshot[occurrence["snapshot_id"]]
        offset = occurrence["raw_value_start"]
        prefix_ratio = ANALYZER.ratio(offset, total_bytes) if offset is not None else None
        gross = occurrence["estimated_tokens"]
        candidates.append({
            "lineage_id": lineage["lineage_id"], "snapshot_id": occurrence["snapshot_id"],
            "block_occurrence_id": occurrence["block_occurrence_id"], "age": occurrence["request_ordinal"] - lineage["first_seen_request"],
            "original_bytes": occurrence["raw_bytes"], "original_estimated_tokens": gross,
            "stub_bytes": stub_bytes, "stub_estimated_tokens": stub_tokens,
            "gross_reduction": gross, "recovery_tool_schema_overhead_share": schema_share,
            "net_shadow_reduction": gross - stub_tokens - schema_share,
            "cache_risk": ANALYZER.risk(prefix_ratio), "first_modified_offset": offset,
            "preserved_prefix_bytes": offset, "preserved_prefix_ratio": prefix_ratio,
            "policy_id": policy_id, "policy_version": POLICY_VERSION,
        })
    return candidates, {"lineage_coverage_exclusions": exclusions, "candidate_limit": limit, "candidates_dropped_by_limit": dropped,
                        "schema_overhead_total": schema_budget, "forwarding_mutations": 0, "shadow_drops": 0}


def markdown(value: dict[str, Any]) -> str:
    summary = value["summary"]
    lines = ["# TRACEPRESS_SHADOW_EVICTION_001", "", "Shadow-only candidate evaluation. Provider forwarding remains unchanged; no recovery object or payload replacement is performed.", "", f"Candidates: **{summary['candidate_count']}**. Forwarding mutations: **0**. Candidates dropped by bound: **{summary['candidates_dropped_by_limit']}**.", "", "| Policy | Candidates | Gross est. tokens | Stub est. tokens | Schema overhead | Net shadow reduction | Cache risk |", "|---|---:|---:|---:|---:|---:|---|"]
    for row in summary["policies"]:
        lines.append(f"| `{row['policy_id']}` | {row['candidate_count']} | {row['gross_reduction']} | {row['stub_overhead']} | {row['schema_overhead']} | {row['net_shadow_reduction']} | `{row['cache_risk_distribution']}` |")
    lines.extend(["", "Privacy: opaque lineage and occurrence identifiers plus bounded metadata only; no raw ToolResult content, content hash, CAS path, or recovery ID is emitted."])
    return "\n".join(lines) + "\n"


def main() -> None:
    args = parse_args()
    selected = args.policy or ["E1", "E2", "E5"]
    rows, snapshots = ANALYZER.load_occurrences(args.database, {}, args.default_workload)
    candidates, integrity = candidate_rows(rows, selected, args.max_candidates)
    policies = []
    for policy_id in selected:
        group = [item for item in candidates if item["policy_id"] == policy_id]
        policies.append({"policy_id": policy_id, "policy_version": POLICY_VERSION, "candidate_count": len(group),
                         "gross_reduction": sum(item["gross_reduction"] for item in group), "stub_overhead": sum(item["stub_estimated_tokens"] for item in group),
                         "schema_overhead": sum(item["recovery_tool_schema_overhead_share"] for item in group), "net_shadow_reduction": sum(item["net_shadow_reduction"] for item in group),
                         "cache_risk_distribution": {risk: sum(item["cache_risk"] == risk for item in group) for risk in ("Low", "Medium", "High", "Unknown") if any(item["cache_risk"] == risk for item in group)}})
    value = {"report_version": 1, "phase": "4.6_shadow_eviction", "shadow_only": True, "runtime_forwarding": "unchanged", "sources": [args.database.name],
             "dataset": {"snapshots": len(snapshots), "blocks": len(rows)}, "candidates": candidates,
             "summary": {"candidate_count": len(candidates), "policies": policies, **integrity},
             "limitations": ["This is a local shadow calculation, not a provider cache measurement.", "Recovery remains unimplemented and is simulated only as stub/schema overhead."]}
    args.output_json.parent.mkdir(parents=True, exist_ok=True)
    args.output_json.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.output_md.write_text(markdown(value), encoding="utf-8")


if __name__ == "__main__":
    main()
