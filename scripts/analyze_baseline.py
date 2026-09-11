#!/usr/bin/env python3
"""Build metadata-only, token-weighted Tracepress baseline reports.

The analyzer reads an existing Tracepress SQLite database and produces a JSON report plus a
compact Markdown rendering. It never emits request/response payloads, identifiers, paths,
fingerprints, headers, or prices. A missing estimate remains missing; it is never converted to
zero and never used to manufacture a reconciliation residual.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import UTC, datetime
import json
import math
import os
from pathlib import Path
import sqlite3
import statistics
import sys
from typing import Any, Iterable


REPORT_VERSION = 1
DEFAULT_MEASUREMENT_ID = "baseline-001"
DEFAULT_COHORT_KIND = "naturalistic"
COHORT_KINDS = ("naturalistic", "compaction_calibration", "mixed")


def percentile(values: Iterable[float | int], percentile_value: float) -> float | None:
    """Return an interpolated percentile, or None for an empty sample."""

    numbers = sorted(float(value) for value in values)
    if not numbers:
        return None
    if len(numbers) == 1:
        return numbers[0]
    position = (len(numbers) - 1) * percentile_value / 100.0
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return numbers[lower]
    fraction = position - lower
    return numbers[lower] + (numbers[upper] - numbers[lower]) * fraction


def distribution(values: Iterable[float | int]) -> dict[str, float | int | None]:
    numbers = [float(value) for value in values]
    return {
        "count": len(numbers),
        "p50": percentile(numbers, 50),
        "p75": percentile(numbers, 75),
        "p90": percentile(numbers, 90),
        "p95": percentile(numbers, 95),
        "p99": percentile(numbers, 99),
        "max": max(numbers) if numbers else None,
    }


def ratio(numerator: int | float | None, denominator: int | float | None) -> float | None:
    if numerator is None or denominator in (None, 0):
        return None
    return float(numerator) / float(denominator)


def percentage(numerator: int | float | None, denominator: int | float | None) -> float | None:
    value = ratio(numerator, denominator)
    return None if value is None else value * 100.0


def jsonable(value: Any) -> Any:
    """Convert SQLite values to report-safe JSON values without exposing binary data."""

    if isinstance(value, (bytes, bytearray, memoryview)):
        return None
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    return str(value)


def table_columns(connection: sqlite3.Connection, table: str) -> set[str]:
    try:
        return {row[1] for row in connection.execute(f"PRAGMA table_info({table})")}
    except sqlite3.DatabaseError:
        return set()


def table_exists(connection: sqlite3.Connection, table: str) -> bool:
    return bool(
        connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?", (table,)
        ).fetchone()
    )


def rows(connection: sqlite3.Connection, table: str, columns: Iterable[str]) -> list[dict[str, Any]]:
    if not table_exists(connection, table):
        return []
    available = table_columns(connection, table)
    selected = []
    for column in columns:
        selected.append(column if column in available else f"NULL AS {column}")
    query = f"SELECT {', '.join(selected)} FROM {table}"
    return [dict(row) for row in connection.execute(query)]


def distinct_values(records: Iterable[dict[str, Any]], key: str) -> Any:
    values = sorted({jsonable(record.get(key)) for record in records if record.get(key) is not None}, key=str)
    if not values:
        return None
    return values[0] if len(values) == 1 else values


def int_value(record: dict[str, Any], key: str) -> int | None:
    value = record.get(key)
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def number_value(record: dict[str, Any], key: str) -> float | None:
    value = record.get(key)
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def first_int(record: dict[str, Any], *keys: str) -> int | None:
    for key in keys:
        value = int_value(record, key)
        if value is not None:
            return value
    return None


def safe_name(value: Any, default: str = "unknown") -> str:
    if value is None or value == "":
        return default
    return str(value)


def aggregate(records: Iterable[dict[str, Any]], key: str, value_key: str = "estimated_tokens") -> list[dict[str, Any]]:
    grouped: dict[str, dict[str, int]] = {}
    for record in records:
        name = safe_name(record.get(key))
        bucket = grouped.setdefault(name, {"block_count": 0, "bytes": 0, "estimated_tokens": 0})
        bucket["block_count"] += 1
        bucket["bytes"] += int_value(record, "raw_bytes") or 0
        bucket["estimated_tokens"] += int_value(record, value_key) or 0
    total_tokens = sum(bucket["estimated_tokens"] for bucket in grouped.values())
    total_bytes = sum(bucket["bytes"] for bucket in grouped.values())
    result = []
    for name, bucket in sorted(grouped.items(), key=lambda item: (-item[1]["estimated_tokens"], item[0])):
        result.append(
            {
                "name": name,
                **bucket,
                "token_share": ratio(bucket["estimated_tokens"], total_tokens),
                "bytes_share": ratio(bucket["bytes"], total_bytes),
            }
        )
    return result


def aggregate_metric_rows(records: Iterable[dict[str, Any]], key: str) -> list[dict[str, Any]]:
    grouped: dict[str, dict[str, int]] = {}
    for record in records:
        name = safe_name(record.get(key))
        bucket = grouped.setdefault(name, {"estimated_tokens": 0, "block_count": 0})
        bucket["estimated_tokens"] += int_value(record, "estimated_tokens") or 0
        bucket["block_count"] += 1
    total = sum(value["estimated_tokens"] for value in grouped.values())
    return [
        {
            "name": name,
            **value,
            "token_share": ratio(value["estimated_tokens"], total),
        }
        for name, value in sorted(grouped.items(), key=lambda item: (-item[1]["estimated_tokens"], item[0]))
    ]


def fingerprint_value(value: Any) -> bytes | None:
    if value is None:
        return None
    if isinstance(value, memoryview):
        return value.tobytes()
    if isinstance(value, bytearray):
        return bytes(value)
    if isinstance(value, bytes):
        return value
    return None


def block_distribution(records: Iterable[dict[str, Any]], key: str) -> dict[str, dict[str, float | int | None]]:
    grouped: dict[str, list[int]] = defaultdict(list)
    for record in records:
        value = int_value(record, key)
        if value is not None:
            grouped[safe_name(record.get("kind"))].append(value)
    return {name: distribution(values) for name, values in sorted(grouped.items())}


def event_counts(
    connection: sqlite3.Connection,
) -> tuple[Counter[str], Counter[str], Counter[str]]:
    counts: Counter[str] = Counter()
    drop_reasons: Counter[str] = Counter()
    drop_work: Counter[str] = Counter()
    event_rows = rows(connection, "events", ("event_type", "payload"))
    for event in event_rows:
        event_type = safe_name(event.get("event_type"))
        counts[event_type] += 1
        if "drop" in event_type or "partial" in event_type:
            payload = event.get("payload")
            if isinstance(payload, memoryview):
                payload = payload.tobytes()
            if isinstance(payload, str):
                payload = payload.encode("utf-8")
            if isinstance(payload, bytes):
                try:
                    decoded = json.loads(payload.decode("utf-8"))
                except (UnicodeDecodeError, json.JSONDecodeError):
                    decoded = {}
                reason = decoded.get("reason") if isinstance(decoded, dict) else None
                if reason:
                    drop_reasons[str(reason)] += 1
                if event_type == "context.analysis.dropped":
                    dropped_count = 1
                    if isinstance(decoded, dict):
                        try:
                            dropped_count = max(int(decoded.get("dropped_count", 1)), 0)
                        except (TypeError, ValueError):
                            dropped_count = 1
                    drop_work[str(reason or "unknown")] += dropped_count
    return counts, drop_reasons, drop_work


def request_attempts(
    requests: list[dict[str, Any]],
    attempts: list[dict[str, Any]],
) -> tuple[dict[str, list[dict[str, Any]]], int]:
    by_request: dict[str, list[dict[str, Any]]] = defaultdict(list)
    errors = 0
    for attempt in attempts:
        by_request[safe_name(attempt.get("request_id"))].append(attempt)
        code = int_value(attempt, "status_code")
        status = safe_name(attempt.get("status"))
        if (code is not None and not 200 <= code < 300) or status not in {"completed", "unknown"}:
            errors += 1
        if attempt.get("transport_error"):
            errors += 1
    known_request_ids = {safe_name(request.get("request_id")) for request in requests}
    errors += sum(1 for request_id in known_request_ids if not by_request.get(request_id))
    return by_request, errors


def repetition_and_persistence(
    blocks: list[dict[str, Any]],
    snapshot_context: dict[str, dict[str, Any]],
) -> tuple[dict[str, Any], dict[str, Any], dict[str, dict[str, Any]]]:
    enriched: list[dict[str, Any]] = []
    for block in blocks:
        context = snapshot_context.get(safe_name(block.get("snapshot_id")), {})
        enriched.append(
            {
                **block,
                "_session_id": context.get("session_id", "unknown"),
                "_snapshot_order": context.get("snapshot_order", 0),
                "_detected": safe_name(block.get("detected_kind")),
                "_estimated": int_value(block, "estimated_tokens"),
            }
        )

    def groups(field: str) -> dict[tuple[str, bytes], list[dict[str, Any]]]:
        result: dict[tuple[str, bytes], list[dict[str, Any]]] = defaultdict(list)
        for block in enriched:
            fingerprint = fingerprint_value(block.get(field))
            if fingerprint is not None:
                result[(block["_session_id"], fingerprint)].append(block)
        return result

    def repeat_stats(field: str) -> dict[str, int | None]:
        repeated_blocks = 0
        repeated_tokens = 0
        within_blocks = 0
        within_tokens = 0
        cross_blocks = 0
        cross_tokens = 0
        for records in groups(field).values():
            records.sort(key=lambda item: (item["_snapshot_order"], int_value(item, "ordinal") or 0))
            repeated = records[1:]
            repeated_blocks += len(repeated)
            repeated_tokens += sum(item["_estimated"] or 0 for item in repeated)
            by_snapshot: Counter[int] = Counter(item["_snapshot_order"] for item in records)
            within_blocks += sum(max(0, count - 1) for count in by_snapshot.values())
            for order, count in by_snapshot.items():
                within_tokens += sum(
                    item["_estimated"] or 0
                    for item in records
                    if item["_snapshot_order"] == order
                ) * (max(0, count - 1) / count if count else 0)
            first_order = records[0]["_snapshot_order"]
            cross = [item for item in records if item["_snapshot_order"] > first_order]
            cross_blocks += len(cross)
            cross_tokens += sum(item["_estimated"] or 0 for item in cross)
        return {
            "repeated_blocks": repeated_blocks,
            "repeated_estimated_tokens": repeated_tokens,
            "within_request_repeated_blocks": within_blocks,
            "within_request_repeated_estimated_tokens": round(within_tokens),
            "cross_request_repeated_blocks": cross_blocks,
            "cross_request_repeated_estimated_tokens": cross_tokens,
        }

    exact = repeat_stats("exact_fingerprint")
    semantic = repeat_stats("semantic_fingerprint")
    estimated_total = sum(block["_estimated"] or 0 for block in enriched)

    fingerprint_records: dict[tuple[str, bytes], list[dict[str, Any]]] = {}
    for field in ("semantic_fingerprint", "exact_fingerprint"):
        for key, records in groups(field).items():
            fingerprint_records.setdefault(key, records)
    persistence_values: list[int] = []
    persistence_by_detected: dict[str, list[int]] = defaultdict(list)
    effective_by_detected: Counter[str] = Counter()
    unique_semantic_by_detected: Counter[str] = Counter()
    repeated_tokens_by_detected: Counter[str] = Counter()
    for records in fingerprint_records.values():
        records.sort(key=lambda item: (item["_snapshot_order"], int_value(item, "ordinal") or 0))
        persistence = len({item["_snapshot_order"] for item in records})
        persistence_values.append(persistence)
        category = records[0]["_detected"]
        persistence_by_detected[category].append(persistence)
        for record in records:
            effective_by_detected[record["_detected"]] += record["_estimated"] or 0
        for record in records[1:]:
            repeated_tokens_by_detected[record["_detected"]] += record["_estimated"] or 0

    for (session_id, fingerprint), records in groups("semantic_fingerprint").items():
        del session_id, fingerprint
        records.sort(key=lambda item: (item["_snapshot_order"], int_value(item, "ordinal") or 0))
        first = records[0]
        if first["_estimated"] is not None:
            unique_semantic_by_detected[first["_detected"]] += first["_estimated"]

    persistence_by_category = {
        name: {
            "distribution": distribution(values),
            "average": statistics.mean(values) if values else None,
        }
        for name, values in sorted(persistence_by_detected.items())
    }
    effective_presence = [
        {
            "name": name,
            "effective_presence_estimated_tokens": tokens,
            "unique_semantic_information_estimated_tokens": unique_semantic_by_detected.get(name, 0),
            "redundancy_factor": ratio(tokens, unique_semantic_by_detected.get(name, 0)),
            "average_persistence": persistence_by_category.get(name, {}).get("average"),
            "repeated_estimated_tokens": repeated_tokens_by_detected.get(name, 0),
            "candidate_redundant_exposure": (
                repeated_tokens_by_detected.get(name, 0)
                * float(persistence_by_category.get(name, {}).get("average") or 1)
            ),
        }
        for name, tokens in sorted(effective_by_detected.items(), key=lambda item: (-item[1], item[0]))
    ]
    repetition = {
        "exact_repeated_blocks": exact["repeated_blocks"],
        "semantic_repeated_blocks": semantic["repeated_blocks"],
        "exact_repeated_estimated_tokens": exact["repeated_estimated_tokens"],
        "semantic_repeated_estimated_tokens": semantic["repeated_estimated_tokens"],
        "within_request": {
            "exact_repeated_blocks": exact["within_request_repeated_blocks"],
            "semantic_repeated_blocks": semantic["within_request_repeated_blocks"],
            "exact_repeated_estimated_tokens": exact["within_request_repeated_estimated_tokens"],
            "semantic_repeated_estimated_tokens": semantic["within_request_repeated_estimated_tokens"],
        },
        "cross_request": {
            "exact_repeated_blocks": exact["cross_request_repeated_blocks"],
            "semantic_repeated_blocks": semantic["cross_request_repeated_blocks"],
            "exact_repeated_estimated_tokens": exact["cross_request_repeated_estimated_tokens"],
            "semantic_repeated_estimated_tokens": semantic["cross_request_repeated_estimated_tokens"],
        },
        "estimated_token_total": estimated_total,
        "exact_repeated_token_share": ratio(exact["repeated_estimated_tokens"], estimated_total),
        "semantic_repeated_token_share": ratio(semantic["repeated_estimated_tokens"], estimated_total),
        "persistence": {
            "distribution": distribution(persistence_values),
            "by_detected_content": persistence_by_category,
        },
        "effective_presence": effective_presence,
    }
    return repetition, {key: value for key, value in persistence_by_category.items()}, {
        name: {
            "effective_presence": item["effective_presence_estimated_tokens"],
            "unique_semantic_information": item["unique_semantic_information_estimated_tokens"],
            "redundancy_factor": item["redundancy_factor"],
            "persistence": item["average_persistence"],
            "repeated_tokens": item["repeated_estimated_tokens"],
        }
        for name, item in ((value["name"], value) for value in effective_presence)
    }


def unknown_report(blocks: list[dict[str, Any]], snapshot_context: dict[str, dict[str, Any]]) -> dict[str, Any]:
    unknown = [block for block in blocks if safe_name(block.get("detected_kind")) == "unknown"]
    estimated = [int_value(block, "estimated_tokens") for block in unknown if int_value(block, "estimated_tokens") is not None]
    total_estimated = sum(int_value(block, "estimated_tokens") or 0 for block in blocks)

    def frequency(field: str) -> list[int]:
        values: Counter[tuple[str, bytes]] = Counter()
        for block in unknown:
            fingerprint = fingerprint_value(block.get(field))
            session_id = safe_name(snapshot_context.get(safe_name(block.get("snapshot_id")), {}).get("session_id"))
            if fingerprint is not None:
                values[(session_id, fingerprint)] += 1
        return list(values.values())

    exact_frequency = frequency("exact_fingerprint")
    semantic_frequency = frequency("semantic_fingerprint")
    return {
        "block_count": len(unknown),
        "estimated_tokens": sum(estimated),
        "token_share": ratio(sum(estimated), total_estimated),
        "raw_bytes": sum(int_value(block, "raw_bytes") or 0 for block in unknown),
        "size_distribution_raw_bytes": distribution(int_value(block, "raw_bytes") or 0 for block in unknown),
        "size_distribution_estimated_tokens": distribution(estimated),
        "exact_fingerprint_unique": len(exact_frequency),
        "exact_fingerprint_max_frequency": max(exact_frequency) if exact_frequency else 0,
        "semantic_fingerprint_unique": len(semantic_frequency),
        "semantic_fingerprint_max_frequency": max(semantic_frequency) if semantic_frequency else 0,
        "semantic_fingerprint_average_frequency": statistics.mean(semantic_frequency) if semantic_frequency else None,
    }


def opportunity_ranking(
    detected: list[dict[str, Any]],
    category_details: dict[str, dict[str, Any]],
    estimator_coverage: float | None,
    semantic_coverage: float | None,
) -> list[dict[str, Any]]:
    controls = {
        "tool_definition": (1.0, 0.95),
        "tool_result": (0.9, 0.85),
        "json": (0.9, 0.8),
        "search_results": (0.9, 0.85),
        "logs": (0.8, 0.7),
        "test_results": (0.8, 0.75),
        "source_code": (0.5, 0.45),
        "diff": (0.5, 0.5),
        "plain_text": (0.7, 0.7),
        "assistant_history": (0.3, 0.3),
        "unknown": (0.2, 0.2),
    }
    max_redundancy = max(
        (float(value.get("redundancy_factor") or 1) for value in category_details.values()),
        default=1.0,
    )
    max_persistence = max((float(value.get("persistence") or 1) for value in category_details.values()), default=1.0)
    ranked = []
    for row in detected:
        name = row["name"]
        details = category_details.get(name, {})
        control, safety = controls.get(name, (0.4, 0.4))
        redundancy = float(details.get("redundancy_factor") or 1)
        persistence = float(details.get("persistence") or 1)
        confidence = (estimator_coverage or 0.0) * (semantic_coverage or 0.0)
        score = (
            float(row.get("token_share") or 0)
            * (redundancy / max_redundancy)
            * (persistence / max_persistence)
            * confidence
            * control
            * safety
        )
        ranked.append(
            {
                "name": name,
                "phase_4_candidate_priority_score": score,
                "estimated_token_volume": row["estimated_tokens"],
                "token_share": row["token_share"],
                "redundancy_factor": details.get("redundancy_factor"),
                "average_persistence": details.get("persistence"),
                "measurement_confidence": confidence,
                "controllability": control,
                "semantic_safety": safety,
            }
        )
    return sorted(ranked, key=lambda row: (-row["phase_4_candidate_priority_score"], row["name"]))


def _snapshot_summary(
    snapshot_id: str,
    blocks_by_snapshot: dict[str, list[dict[str, Any]]],
    usage_by_request: dict[str, int],
    snapshot_context: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    blocks = blocks_by_snapshot.get(snapshot_id, [])
    estimated = [int_value(block, "estimated_tokens") for block in blocks]
    estimated = [value for value in estimated if value is not None]
    by_kind = aggregate(blocks, "kind")
    by_detected = aggregate(blocks, "detected_kind")
    request_id = safe_name(snapshot_context.get(snapshot_id, {}).get("request_id"))
    return {
        "visible_estimated_tokens": sum(estimated) if estimated else None,
        "provider_input_tokens": usage_by_request.get(request_id),
        "by_context_block_kind": by_kind,
        "detected_content": by_detected,
    }


def _survival_by_category(
    before: list[dict[str, Any]],
    after: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Match anonymous fingerprint multisets without exposing fingerprint bytes."""

    after_by_key: dict[tuple[str, bytes], list[int]] = defaultdict(list)
    for block in after:
        fingerprint = fingerprint_value(block.get("semantic_fingerprint"))
        field = "semantic" if fingerprint is not None else "exact"
        if fingerprint is None:
            fingerprint = fingerprint_value(block.get("exact_fingerprint"))
        if fingerprint is not None:
            after_by_key[(field, fingerprint)].append(int_value(block, "estimated_tokens") or 0)
    for values in after_by_key.values():
        values.sort()

    totals: Counter[str] = Counter()
    survived: Counter[str] = Counter()
    for block in before:
        estimated = int_value(block, "estimated_tokens")
        category = safe_name(block.get("detected_kind"))
        if estimated is None:
            continue
        totals[category] += estimated
        fingerprint = fingerprint_value(block.get("semantic_fingerprint"))
        field = "semantic" if fingerprint is not None else "exact"
        if fingerprint is None:
            fingerprint = fingerprint_value(block.get("exact_fingerprint"))
        if fingerprint is None:
            continue
        candidates = after_by_key.get((field, fingerprint), [])
        if candidates:
            candidates.pop()
            survived[category] += estimated
    return [
        {
            "name": category,
            "before_estimated_tokens": total,
            "survived_estimated_tokens": survived.get(category, 0),
            "survival_share": ratio(survived.get(category, 0), total),
        }
        for category, total in sorted(totals.items(), key=lambda item: (-item[1], item[0]))
    ]


def compaction_report(
    requests: list[dict[str, Any]],
    attempts_by_request: dict[str, list[dict[str, Any]]],
    usage_by_request: dict[str, int],
    snapshots: list[dict[str, Any]],
    blocks_by_snapshot: dict[str, list[dict[str, Any]]],
    snapshot_context: dict[str, dict[str, Any]],
    cohort_kind: str,
) -> dict[str, Any]:
    compaction_kinds = {"compaction_v2", "compaction_legacy"}
    compactions = [request for request in requests if safe_name(request.get("request_kind")) in compaction_kinds]
    compaction_ids = {safe_name(request.get("request_id")) for request in compactions}
    output_seen = sum(
        1
        for request in compactions
        if any(
            int_value(attempt, "compaction_output_seen") == 1
            for attempt in attempts_by_request.get(safe_name(request.get("request_id")), [])
        )
    )
    trigger_seen = sum(1 for request in compactions if request.get("compaction_trigger") is not None)
    ordered_by_session: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for snapshot in snapshots:
        ordered_by_session[safe_name(snapshot.get("session_id"))].append(snapshot)
    for values in ordered_by_session.values():
        values.sort(key=lambda row: snapshot_context.get(safe_name(row.get("snapshot_id")), {}).get("snapshot_order", 0))

    pairs: list[dict[str, Any]] = []
    windows: list[dict[str, Any]] = []
    session_ordinal = 0
    for session_snapshots in ordered_by_session.values():
        compaction_positions = [
            index
            for index, snapshot in enumerate(session_snapshots)
            if safe_name(snapshot_context.get(safe_name(snapshot.get("snapshot_id")), {}).get("request_id")) in compaction_ids
        ]
        if not compaction_positions:
            continue
        session_ordinal += 1
        boundaries = [-1, *compaction_positions, len(session_snapshots)]
        for window_index, (start, end) in enumerate(zip(boundaries, boundaries[1:])):
            window_snapshots = session_snapshots[start + 1 : end]
            summaries = [
                _snapshot_summary(safe_name(snapshot.get("snapshot_id")), blocks_by_snapshot, usage_by_request, snapshot_context)
                for snapshot in window_snapshots
            ]
            visible = [summary["visible_estimated_tokens"] for summary in summaries if summary["visible_estimated_tokens"] is not None]
            provider_input = [summary["provider_input_tokens"] for summary in summaries if summary["provider_input_tokens"] is not None]
            windows.append(
                {
                    "session_ordinal": session_ordinal,
                    "window_index": window_index,
                    "requests": len(window_snapshots),
                    "visible_estimated_tokens": sum(visible) if visible else None,
                    "provider_input_tokens": sum(provider_input) if provider_input else None,
                }
            )
        for position in compaction_positions:
            before = next(
                (session_snapshots[index] for index in range(position - 1, -1, -1)
                 if safe_name(snapshot_context.get(safe_name(session_snapshots[index].get("snapshot_id")), {}).get("request_id")) not in compaction_ids),
                None,
            )
            after = next(
                (session_snapshots[index] for index in range(position + 1, len(session_snapshots))
                 if safe_name(snapshot_context.get(safe_name(session_snapshots[index].get("snapshot_id")), {}).get("request_id")) not in compaction_ids),
                None,
            )
            if before is None or after is None:
                continue
            before_id = safe_name(before.get("snapshot_id"))
            after_id = safe_name(after.get("snapshot_id"))
            before_summary = _snapshot_summary(before_id, blocks_by_snapshot, usage_by_request, snapshot_context)
            after_summary = _snapshot_summary(after_id, blocks_by_snapshot, usage_by_request, snapshot_context)
            before_visible = before_summary["visible_estimated_tokens"]
            after_visible = after_summary["visible_estimated_tokens"]
            pairs.append(
                {
                    "pre": before_summary,
                    "post": after_summary,
                    "observed_context_reduction": (
                        ratio(before_visible - after_visible, before_visible)
                        if before_visible is not None and after_visible is not None and before_visible
                        else None
                    ),
                    "survival_by_category": _survival_by_category(
                        blocks_by_snapshot.get(before_id, []), blocks_by_snapshot.get(after_id, [])
                    ),
                }
            )
    reductions = [pair["observed_context_reduction"] for pair in pairs if pair["observed_context_reduction"] is not None]
    return {
        "cohort_kind": cohort_kind,
        "requests": len(compactions),
        "v2_requests": sum(1 for request in compactions if request.get("request_kind") == "compaction_v2"),
        "legacy_requests": sum(1 for request in compactions if request.get("request_kind") == "compaction_legacy"),
        "trigger_seen": trigger_seen,
        "trigger_values": dict(Counter(safe_name(request.get("compaction_trigger")) for request in compactions if request.get("compaction_trigger") is not None)),
        "output_seen": output_seen,
        "pre_post_pairs": len(pairs),
        "observed_context_reduction": distribution(reductions),
        "pre_post": pairs,
        "survival_by_category": (
            [
                {
                    "name": name,
                    "survival_share": ratio(
                        sum(row["survived_estimated_tokens"] for pair in pairs for row in pair["survival_by_category"] if row["name"] == name),
                        sum(row["before_estimated_tokens"] for pair in pairs for row in pair["survival_by_category"] if row["name"] == name),
                    ),
                }
                for name in sorted({row["name"] for pair in pairs for row in pair["survival_by_category"]})
            ]
            if pairs
            else []
        ),
        "windows": windows,
    }


def analyze_connection(
    connection: sqlite3.Connection,
    *,
    measurement_id: str = DEFAULT_MEASUREMENT_ID,
    cohort_label: str = "n20",
    cohort_kind: str = DEFAULT_COHORT_KIND,
    tracepress_commit: str = "unknown",
    codex_version: str = "unknown",
) -> dict[str, Any]:
    connection.row_factory = sqlite3.Row
    session_rows = rows(connection, "sessions", ("session_id", "state", "ended_at"))
    operation_rows = rows(connection, "operations", ("operation_id", "session_id", "kind", "status"))
    request_rows = rows(
        connection,
        "provider_requests",
        (
            "request_id",
            "operation_id",
            "request_kind",
            "transport",
            "model",
            "reasoning_effort",
            "parser_version",
            "observation_status",
            "content_encoding",
            "analysis_decode_status",
            "wire_bytes",
            "decoded_bytes",
            "compaction_trigger",
        ),
    )
    attempt_rows = rows(
        connection,
        "provider_attempts",
        (
            "attempt_id",
            "request_id",
            "status_code",
            "status",
            "observation_status",
            "transport_error",
            "compaction_output_seen",
        ),
    )
    usage_rows = rows(
        connection,
        "provider_usage",
        (
            "attempt_id",
            "input_total",
            "input_cached",
            "output_total",
            "output_reasoning",
            "usage_status",
            "normalizer_version",
        ),
    )
    snapshot_rows = rows(
        connection,
        "context_snapshots",
        (
            "snapshot_id",
            "session_id",
            "provider_request_id",
            "analysis_version",
            "status",
            "started_at_us",
            "completed_at_us",
            "correlation_status",
            "explicit_request_complete",
        ),
    )
    metric_rows = rows(
        connection,
        "context_analysis_metrics",
        (
            "snapshot_id",
            "unknown_block_count",
            "semantic_coverage_basis_points",
            "stable_explicit_prefix_estimate",
            "estimator",
            "estimator_version",
            "estimate_confidence",
        ),
    )
    block_rows = rows(
        connection,
        "context_block_occurrences",
        (
            "block_occurrence_id",
            "snapshot_id",
            "ordinal",
            "kind",
            "role",
            "origin",
            "raw_bytes",
            "exact_fingerprint",
            "semantic_fingerprint",
            "fingerprint_version",
            "estimated_tokens",
            "estimator",
            "estimator_version",
            "estimate_confidence",
            "detected_kind",
            "detector_version",
            "repetition_score",
        ),
    )
    delta_rows = rows(
        connection,
        "context_deltas",
        (
            "current_snapshot_id",
            "repeated_blocks",
            "new_blocks",
            "changed_blocks",
            "removed_blocks",
            "repeated_estimated_tokens",
            "new_estimated_tokens",
            "common_prefix_estimated_tokens",
        ),
    )
    reconciliation_rows = rows(
        connection,
        "token_reconciliations",
        ("snapshot_id", "visible_estimated_tokens", "provider_input_tokens", "residual_tokens", "comparability"),
    )
    counts, drop_reasons, drop_work = event_counts(connection)
    attempts_by_request, forwarding_errors = request_attempts(request_rows, attempt_rows)

    operation_session = {safe_name(row.get("operation_id")): safe_name(row.get("session_id")) for row in operation_rows}
    request_session = {safe_name(row.get("request_id")): operation_session.get(safe_name(row.get("operation_id")), "unknown") for row in request_rows}
    snapshot_context: dict[str, dict[str, Any]] = {}
    snapshots_by_session: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for snapshot in snapshot_rows:
        session_id = safe_name(snapshot.get("session_id"))
        snapshots_by_session[session_id].append(snapshot)
    for session_id, session_snapshots in snapshots_by_session.items():
        session_snapshots.sort(
            key=lambda snapshot: (
                int_value(snapshot, "started_at_us") is None,
                int_value(snapshot, "started_at_us") or 0,
                safe_name(snapshot.get("snapshot_id")),
            )
        )
        for order, snapshot in enumerate(session_snapshots):
            snapshot_context[safe_name(snapshot.get("snapshot_id"))] = {
                "session_id": session_id,
                "snapshot_order": order,
                "request_id": safe_name(snapshot.get("provider_request_id")),
            }

    by_snapshot_metric = {safe_name(row.get("snapshot_id")): row for row in metric_rows}
    by_snapshot_blocks: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for block in block_rows:
        by_snapshot_blocks[safe_name(block.get("snapshot_id"))].append(block)
    block_estimates = [int_value(block, "estimated_tokens") for block in block_rows]
    estimated_block_values = [value for value in block_estimates if value is not None]
    estimated_token_total = sum(estimated_block_values)
    raw_token_total = sum(int_value(block, "raw_bytes") or 0 for block in block_rows)

    analysis_started = counts.get("context.analysis.started", len(snapshot_rows))
    analysis_complete = sum(1 for row in snapshot_rows if row.get("status") == "complete")
    analysis_partial = sum(1 for row in snapshot_rows if row.get("status") == "partial")
    analysis_dropped = sum(drop_work.values())
    if not analysis_dropped:
        analysis_dropped = counts.get("context.analysis.dropped", 0)
    analysis_seen = analysis_started + analysis_dropped
    if analysis_seen < analysis_complete + analysis_partial + analysis_dropped:
        analysis_seen = analysis_complete + analysis_partial + analysis_dropped
    correlation_eligible = sum(1 for row in snapshot_rows if row.get("correlation_status") is not None)
    correlation_correlated = sum(1 for row in snapshot_rows if row.get("correlation_status") == "correlated")
    semantic_values = [int_value(row, "semantic_coverage_basis_points") for row in metric_rows]
    semantic_values = [value for value in semantic_values if value is not None]
    malformed = counts.get("context.analysis.malformed", 0) + drop_reasons.get("malformed", 0)

    final_usage = [row for row in usage_rows if row.get("usage_status") == "final"]
    input_total_values = [first_int(row, "input_total") for row in final_usage]
    input_total_values = [value for value in input_total_values if value is not None]
    cached_values = [first_int(row, "input_cached", "cache_read") for row in final_usage]
    cached_values = [value for value in cached_values if value is not None]
    output_values = [first_int(row, "output_total") for row in final_usage]
    output_values = [value for value in output_values if value is not None]
    reasoning_values = [first_int(row, "output_reasoning", "reasoning") for row in final_usage]
    reasoning_values = [value for value in reasoning_values if value is not None]
    reconciliation_unavailable = Counter(
        safe_name(row.get("comparability"))
        for row in reconciliation_rows
        if row.get("residual_tokens") is None
    )
    residuals = [int_value(row, "residual_tokens") for row in reconciliation_rows]
    residuals = [value for value in residuals if value is not None]

    detected_content = aggregate(block_rows, "detected_kind")
    composition = {
        "by_context_block_kind": aggregate(block_rows, "kind"),
        "by_role": aggregate(block_rows, "role"),
        "by_origin": aggregate(block_rows, "origin"),
        "detected_content": detected_content,
        "estimated_token_total": estimated_token_total,
        "raw_bytes_total": raw_token_total,
    }
    repetition, _persistence, category_details = repetition_and_persistence(block_rows, snapshot_context)
    metric_prefix_values = [int_value(row, "stable_explicit_prefix_estimate") for row in metric_rows]
    metric_prefix_values = [value for value in metric_prefix_values if value is not None]
    stable_prefix_total = sum(metric_prefix_values)
    stable_prefix = {
        "estimated_tokens": stable_prefix_total,
        "mean_estimated_tokens": statistics.mean(metric_prefix_values) if metric_prefix_values else None,
        "share": ratio(stable_prefix_total, estimated_token_total),
        "provider_cache_comparison": "descriptive_only",
    }
    unknown = unknown_report(block_rows, snapshot_context)
    estimator_coverage = ratio(len(estimated_block_values), len(block_rows))
    semantic_coverage = ratio(sum(semantic_values), len(semantic_values) * 10_000) if semantic_values else None
    ranking = opportunity_ranking(detected_content, category_details, estimator_coverage, semantic_coverage)
    request_kinds = Counter(safe_name(row.get("request_kind")) for row in request_rows)
    usage_cache_ratio = ratio(sum(cached_values), sum(input_total_values)) if input_total_values and cached_values else None
    attempts_with_usage = {safe_name(row.get("attempt_id")) for row in final_usage}
    usage_by_attempt = {safe_name(row.get("attempt_id")): first_int(row, "input_total") for row in final_usage}
    usage_by_request = {
        safe_name(attempt.get("request_id")): usage_by_attempt.get(safe_name(attempt.get("attempt_id")))
        for attempt in attempt_rows
        if safe_name(attempt.get("attempt_id")) in usage_by_attempt
        and usage_by_attempt.get(safe_name(attempt.get("attempt_id"))) is not None
    }
    usage_final_count = sum(
        1
        for attempt in attempt_rows
        if safe_name(attempt.get("attempt_id")) in attempts_with_usage
    )
    distinct_statuses = Counter(safe_name(row.get("status")) for row in snapshot_rows)

    report = {
        "report_version": REPORT_VERSION,
        "generated_at_utc": datetime.now(UTC).isoformat(),
        "measurement_id": measurement_id,
        "cohort_label": cohort_label,
        "manifest": {
            "tracepress_commit": tracepress_commit,
            "codex_version": codex_version,
            "model": distinct_values(request_rows, "model"),
            "reasoning_effort": distinct_values(request_rows, "reasoning_effort"),
            "transport": distinct_values(request_rows, "transport"),
            "analysis_version": distinct_values(snapshot_rows, "analysis_version"),
            "provider_parser_version": distinct_values(request_rows, "parser_version"),
            "usage_normalizer_version": distinct_values(usage_rows, "normalizer_version"),
            "fingerprint_version": distinct_values(block_rows, "fingerprint_version"),
            "detector_version": distinct_values(block_rows, "detector_version"),
            "token_estimator_version": distinct_values(block_rows, "estimator_version"),
        },
        "dataset": {
            "sessions_total": len(session_rows),
            "sessions_closed": sum(1 for row in session_rows if row.get("state") == "closed"),
            "sessions_by_state": dict(Counter(safe_name(row.get("state")) for row in session_rows)),
            "requests_total": len(request_rows),
            "requests_per_session": distribution(
                len([request for request in request_rows if request_session.get(safe_name(request.get("request_id"))) == session_id])
                for session_id in {safe_name(row.get("session_id")) for row in session_rows}
            ),
            "request_kinds": dict(request_kinds),
            "request_observation_status": dict(Counter(safe_name(row.get("observation_status")) for row in request_rows)),
        },
        "quality": {
            "analysis_requests_seen": analysis_seen,
            "analysis_complete": analysis_complete,
            "analysis_partial": analysis_partial,
            "analysis_dropped": analysis_dropped,
            "analysis_coverage": ratio(analysis_complete, analysis_seen),
            "correlation_eligible": correlation_eligible,
            "correlation_correlated": correlation_correlated,
            "correlation_coverage": ratio(correlation_correlated, correlation_eligible),
            "semantic_coverage": {
                "mean": ratio(sum(semantic_values), len(semantic_values) * 10_000) if semantic_values else None,
                "min": min(semantic_values) / 10_000 if semantic_values else None,
                "max": max(semantic_values) / 10_000 if semantic_values else None,
                "p50": (percentile(semantic_values, 50) or 0) / 10_000 if semantic_values else None,
                "p90": (percentile(semantic_values, 90) or 0) / 10_000 if semantic_values else None,
                "p95": (percentile(semantic_values, 95) or 0) / 10_000 if semantic_values else None,
                "observations": len(semantic_values),
            },
            "token_estimation_coverage": {
                "eligible_blocks": len(block_rows),
                "observed_blocks": len(estimated_block_values),
                "by_blocks": estimator_coverage,
                "estimated_token_subset": estimated_token_total,
            },
            "forwarding_errors": forwarding_errors,
            "context_malformed": malformed,
            "provider_usage_final": usage_final_count,
            "provider_observation_partial": counts.get("provider.observation.partial", 0),
            "snapshot_statuses": dict(distinct_statuses),
            "event_drop_reasons": dict(drop_reasons),
            "event_drop_work": dict(drop_work),
        },
        "provider_usage": {
            "rows": len(usage_rows),
            "final_rows": len(final_usage),
            "input_total": sum(input_total_values) if input_total_values else None,
            "cached_input": sum(cached_values) if cached_values else None,
            "uncached_input": (
                sum(input_total_values) - sum(cached_values)
                if input_total_values and cached_values and sum(input_total_values) >= sum(cached_values)
                else None
            ),
            "output_total": sum(output_values) if output_values else None,
            "reasoning_output": sum(reasoning_values) if reasoning_values else None,
            "cache_ratio": usage_cache_ratio,
            "token_reconciliation_available": bool(residuals),
            "reconciliation_rows": len(reconciliation_rows),
            "reconciliation_unavailable": dict(reconciliation_unavailable),
            "residual_distribution": distribution(residuals),
            "billing_model": "unknown",
            "cost_usd": None,
        },
        "composition": composition,
        "distributions": {
            "raw_bytes": distribution(int_value(block, "raw_bytes") or 0 for block in block_rows),
            "estimated_tokens": distribution(estimated_block_values),
            "estimated_tokens_by_context_kind": block_distribution(block_rows, "estimated_tokens"),
            "raw_bytes_by_context_kind": block_distribution(block_rows, "raw_bytes"),
        },
        "unknown": unknown,
        "repetition": repetition,
        "stable_prefix": stable_prefix,
        "compaction": compaction_report(
            request_rows,
            attempts_by_request,
            usage_by_request,
            snapshot_rows,
            by_snapshot_blocks,
            snapshot_context,
            cohort_kind,
        ),
        "opportunity_ranking": ranking,
        "context_analysis_metrics": {
            "rows": len(metric_rows),
            "estimated_tool_definition_share_mean": statistics.mean(
                [number_value(row, "estimated_tool_definition_share") for row in metric_rows if number_value(row, "estimated_tool_definition_share") is not None]
            )
            if any(number_value(row, "estimated_tool_definition_share") is not None for row in metric_rows)
            else None,
            "estimated_tool_result_share_mean": statistics.mean(
                [number_value(row, "estimated_tool_result_share") for row in metric_rows if number_value(row, "estimated_tool_result_share") is not None]
            )
            if any(number_value(row, "estimated_tool_result_share") is not None for row in metric_rows)
            else None,
        },
        "repetition_deltas": {
            "rows": len(delta_rows),
            "repeated_blocks": sum(int_value(row, "repeated_blocks") or 0 for row in delta_rows),
            "new_blocks": sum(int_value(row, "new_blocks") or 0 for row in delta_rows),
            "changed_blocks": sum(int_value(row, "changed_blocks") or 0 for row in delta_rows),
            "removed_blocks": sum(int_value(row, "removed_blocks") or 0 for row in delta_rows),
            "repeated_estimated_tokens": sum(int_value(row, "repeated_estimated_tokens") or 0 for row in delta_rows)
            if any(row.get("repeated_estimated_tokens") is not None for row in delta_rows)
            else None,
            "new_estimated_tokens": sum(int_value(row, "new_estimated_tokens") or 0 for row in delta_rows)
            if any(row.get("new_estimated_tokens") is not None for row in delta_rows)
            else None,
            "stable_prefix_estimated_tokens": sum(int_value(row, "common_prefix_estimated_tokens") or 0 for row in delta_rows)
            if any(row.get("common_prefix_estimated_tokens") is not None for row in delta_rows)
            else None,
        },
        "limitations": [
            "token estimates are heuristic",
            "Subscription has no API-cost semantics",
            "provider-managed context may be partially invisible",
            "candidate exposure is not saveable tokens",
            "cost is intentionally unavailable",
        ],
    }
    return json.loads(json.dumps(report, default=jsonable))


def format_number(value: Any) -> str:
    if value is None:
        return "unknown"
    if isinstance(value, float):
        return f"{value:.4f}" if not value.is_integer() else str(int(value))
    return f"{value:,}" if isinstance(value, int) else str(value)


def format_pct(value: Any) -> str:
    return "unknown" if value is None else f"{float(value) * 100:.2f}%"


def render_markdown(report: dict[str, Any]) -> str:
    manifest = report["manifest"]
    dataset = report["dataset"]
    quality = report["quality"]
    usage = report["provider_usage"]
    lines = [
        f"# Tracepress Baseline {report['cohort_label']}",
        "",
        "Metadata-only report. No request/response payloads, identifiers, fingerprints, headers, or prices are included.",
        "",
        "## Manifest",
        "",
        *[f"- `{key}`: {format_number(value)}" for key, value in manifest.items()],
        "",
        "## Dataset",
        "",
        f"- Sessions: {format_number(dataset['sessions_total'])} total; {format_number(dataset['sessions_closed'])} closed.",
        f"- Requests: {format_number(dataset['requests_total'])}.",
        f"- Request kinds: `{json.dumps(dataset['request_kinds'], sort_keys=True)}`.",
        "",
        "## Measurement quality",
        "",
        f"- Analysis: {format_pct(quality['analysis_coverage'])} ({quality['analysis_complete']}/{quality['analysis_requests_seen']}).",
        f"- Correlation: {format_pct(quality['correlation_coverage'])} ({quality['correlation_correlated']}/{quality['correlation_eligible']}).",
        f"- Forwarding errors: {format_number(quality['forwarding_errors'])}.",
        f"- Context malformed: {format_number(quality['context_malformed'])}.",
        f"- Token-estimation coverage by blocks: {format_pct(quality['token_estimation_coverage']['by_blocks'])}.",
        f"- Semantic coverage mean: {format_pct(quality['semantic_coverage']['mean'])}.",
        f"- Provider observation partial: {format_number(quality['provider_observation_partial'])}.",
        "",
        "## Provider usage",
        "",
        f"- Input: {format_number(usage['input_total'])}; cached: {format_number(usage['cached_input'])}; uncached: {format_number(usage['uncached_input'])}.",
        f"- Output: {format_number(usage['output_total'])}; reasoning: {format_number(usage['reasoning_output'])}.",
        f"- Cache ratio: {format_pct(usage['cache_ratio'])}.",
        f"- Reconciliation available: `{usage['token_reconciliation_available']}`; unavailable reasons: `{json.dumps(usage['reconciliation_unavailable'], sort_keys=True)}`.",
        "",
        "## Token-weighted composition",
        "",
        "| Category | Estimated tokens | Token share | Blocks |",
        "| --- | ---: | ---: | ---: |",
    ]
    for row in report["composition"]["detected_content"]:
        lines.append(f"| {row['name']} | {format_number(row['estimated_tokens'])} | {format_pct(row['token_share'])} | {row['block_count']} |")
    lines.extend(
        [
            "",
            "## Repetition and exposure",
            "",
            f"- Exact repeated blocks: {format_number(report['repetition']['exact_repeated_blocks'])}; semantic: {format_number(report['repetition']['semantic_repeated_blocks'])}.",
            f"- Exact repeated token share: {format_pct(report['repetition']['exact_repeated_token_share'])}; semantic: {format_pct(report['repetition']['semantic_repeated_token_share'])}.",
            f"- Stable explicit-prefix estimate: {format_number(report['stable_prefix']['estimated_tokens'])}; share: {format_pct(report['stable_prefix']['share'])}.",
            f"- Unknown detected content: {format_number(report['unknown']['estimated_tokens'])} estimated tokens ({format_pct(report['unknown']['token_share'])}).",
            "",
            "## Compaction",
            "",
            f"- Cohort: `{report['compaction']['cohort_kind']}`; requests: {report['compaction']['requests']}; V2: {report['compaction']['v2_requests']}; legacy: {report['compaction']['legacy_requests']}.",
            f"- Trigger seen: {report['compaction']['trigger_seen']}; output seen: {report['compaction']['output_seen']}.",
            "",
            "## Phase 4 candidate priority",
            "",
            "This is a ranking signal, not an expected savings percentage.",
            "",
            "| Rank | Category | Score | Token share | Redundancy | Persistence |",
            "| ---: | --- | ---: | ---: | ---: | ---: |",
        ]
    )
    for index, row in enumerate(report["opportunity_ranking"][:10], start=1):
        redundancy = format_number(row["redundancy_factor"])
        persistence = format_number(row["average_persistence"])
        lines.append(f"| {index} | {row['name']} | {row['phase_4_candidate_priority_score']:.6f} | {format_pct(row['token_share'])} | {redundancy} | {persistence} |")
    lines.extend(["", "## Limitations", "", *[f"- {item}." for item in report["limitations"]], ""])
    return "\n".join(lines)


def default_database() -> Path:
    configured = os.environ.get("TRACEPRESS_DATABASE")
    if configured:
        return Path(configured)
    root = Path(os.environ.get("TRACEPRESS_HOME", ".tracepress"))
    return root / "tracepress.sqlite3"


def write_report_artifacts(report: dict[str, Any], output_dir: Path) -> tuple[Path, Path, Path]:
    """Write the report, Markdown rendering, and immutable experiment manifest."""

    output_dir.mkdir(parents=True, exist_ok=True)
    cohort_label = safe_name(report.get("cohort_label"), "unknown")
    json_path = output_dir / f"baseline_{cohort_label}.json"
    markdown_path = output_dir / f"baseline_{cohort_label}.md"
    manifest_path = output_dir / "measurement_manifest.json"
    json_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    markdown_path.write_text(render_markdown(report), encoding="utf-8")
    manifest = {
        "measurement_id": report.get("measurement_id"),
        "cohort_label": report.get("cohort_label"),
        "cohort_kind": report.get("compaction", {}).get("cohort_kind"),
        **report.get("manifest", {}),
    }
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return json_path, markdown_path, manifest_path


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, default=default_database())
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--measurement-id", default=DEFAULT_MEASUREMENT_ID)
    parser.add_argument("--cohort-label", required=True, help="Report label, for example n20 or n30")
    parser.add_argument("--cohort-kind", choices=COHORT_KINDS, default=DEFAULT_COHORT_KIND)
    parser.add_argument("--tracepress-commit", default=os.environ.get("TRACEPRESS_COMMIT", "unknown"))
    parser.add_argument("--codex-version", default=os.environ.get("CODEX_VERSION", "unknown"))
    return parser


def main(arguments: list[str] | None = None) -> int:
    parser = build_parser()
    options = parser.parse_args(arguments)
    if not options.database.is_file():
        parser.error(f"database does not exist: {options.database}")
    try:
        database_uri = f"file:{options.database.resolve()}?mode=ro"
        connection = sqlite3.connect(database_uri, uri=True)
        report = analyze_connection(
            connection,
            measurement_id=options.measurement_id,
            cohort_label=options.cohort_label,
            cohort_kind=options.cohort_kind,
            tracepress_commit=options.tracepress_commit,
            codex_version=options.codex_version,
        )
    except sqlite3.DatabaseError as error:
        print(f"baseline analysis failed: {error}", file=sys.stderr)
        return 1
    finally:
        if "connection" in locals():
            connection.close()
    json_path, markdown_path, manifest_path = write_report_artifacts(report, options.output_dir)
    print(f"baseline report written: {json_path}")
    print(f"baseline report written: {markdown_path}")
    print(f"measurement manifest written: {manifest_path}")
    print(
        "quality: "
        f"analysis={format_pct(report['quality']['analysis_coverage'])} "
        f"correlation={format_pct(report['quality']['correlation_coverage'])} "
        f"forwarding_errors={report['quality']['forwarding_errors']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
