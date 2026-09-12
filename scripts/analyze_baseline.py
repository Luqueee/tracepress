#!/usr/bin/env python3
"""Build metadata-only, token-weighted Tracepress baseline reports.

The analyzer reads an existing Tracepress SQLite database and produces a JSON report plus a
compact Markdown rendering. It never emits request/response payloads, raw fingerprints, headers,
or prices. The accounting ledger may contain opaque durable IDs so outcomes can be audited by
request. A missing estimate remains missing; it is never converted to zero and never used to
manufacture a reconciliation residual.
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
DEFAULT_MEASUREMENT_INSTRUMENT_VERSION = 2
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


def estimation_coverage_by(records: Iterable[dict[str, Any]], key: str) -> list[dict[str, Any]]:
    """Show which categories contribute estimable blocks and bytes."""

    grouped: dict[str, dict[str, int]] = {}
    for record in records:
        name = safe_name(record.get(key))
        bucket = grouped.setdefault(
            name,
            {
                "block_count": 0,
                "estimated_block_count": 0,
                "raw_bytes": 0,
                "estimated_raw_bytes": 0,
                "estimated_tokens": 0,
            },
        )
        bucket["block_count"] += 1
        raw_bytes = int_value(record, "raw_bytes") or 0
        bucket["raw_bytes"] += raw_bytes
        estimate = int_value(record, "estimated_tokens")
        if estimate is not None:
            bucket["estimated_block_count"] += 1
            bucket["estimated_raw_bytes"] += raw_bytes
            bucket["estimated_tokens"] += estimate
    total_estimated_tokens = sum(bucket["estimated_tokens"] for bucket in grouped.values())
    return [
        {
            "name": name,
            **bucket,
            "block_coverage": ratio(bucket["estimated_block_count"], bucket["block_count"]),
            "bytes_coverage": ratio(bucket["estimated_raw_bytes"], bucket["raw_bytes"]),
            "estimated_token_share": ratio(bucket["estimated_tokens"], total_estimated_tokens),
        }
        for name, bucket in sorted(grouped.items(), key=lambda item: (-item[1]["estimated_tokens"], item[0]))
    ]


def aggregate_cross(
    records: Iterable[dict[str, Any]],
    keys: tuple[str, ...],
) -> list[dict[str, Any]]:
    """Aggregate token and byte composition over a bounded metadata cross-tab."""

    grouped: dict[tuple[str, ...], dict[str, int]] = {}
    for record in records:
        values = tuple(safe_name(record.get(key)) for key in keys)
        bucket = grouped.setdefault(values, {"block_count": 0, "raw_bytes": 0, "estimated_tokens": 0})
        bucket["block_count"] += 1
        bucket["raw_bytes"] += int_value(record, "raw_bytes") or 0
        bucket["estimated_tokens"] += int_value(record, "estimated_tokens") or 0
    total_tokens = sum(bucket["estimated_tokens"] for bucket in grouped.values())
    total_bytes = sum(bucket["raw_bytes"] for bucket in grouped.values())
    return [
        {
            **dict(zip(keys, values)),
            "name": " | ".join(f"{key}={value}" for key, value in zip(keys, values)),
            **bucket,
            "token_share": ratio(bucket["estimated_tokens"], total_tokens),
            "bytes_share": ratio(bucket["raw_bytes"], total_bytes),
        }
        for values, bucket in sorted(grouped.items(), key=lambda item: (-item[1]["estimated_tokens"], item[0]))
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
    session_ids: set[str] | None = None,
) -> tuple[Counter[str], Counter[str], Counter[str]]:
    counts: Counter[str] = Counter()
    drop_reasons: Counter[str] = Counter()
    drop_work: Counter[str] = Counter()
    event_rows = rows(connection, "events", ("session_id", "event_type", "payload"))
    for event in event_rows:
        if session_ids is not None:
            payload = _payload_object(event.get("payload"))
            event_session_id = _nullable_name(payload.get("session_id")) or _nullable_name(
                event.get("session_id")
            )
            if event_session_id not in session_ids:
                continue
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


# Context snapshots are produced for normal Responses turns. Compaction transports are tracked
# as provider observations by Phase 3.1.2, but are intentionally not interpreted as ordinary
# context-analysis requests until a dedicated compaction snapshot contract exists.
ELIGIBLE_REQUEST_KINDS = {"turn"}
def _nullable_name(value: Any) -> str | None:
    if value is None or value == "":
        return None
    return str(value)


def _payload_object(value: Any) -> dict[str, Any]:
    if isinstance(value, memoryview):
        value = value.tobytes()
    if isinstance(value, bytes):
        try:
            value = value.decode("utf-8")
        except UnicodeDecodeError:
            return {}
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except json.JSONDecodeError:
            return {}
    return value if isinstance(value, dict) else {}


def _payload_ids(payload: dict[str, Any], *keys: str) -> list[str]:
    values: list[str] = []
    for key in keys:
        value = payload.get(key)
        candidates = value if isinstance(value, list) else [value]
        for candidate in candidates:
            name = _nullable_name(candidate)
            if name is not None and name not in values:
                values.append(name)
    nested_request = payload.get("request")
    if isinstance(nested_request, dict):
        for key in ("request_id", "provider_request_id"):
            name = _nullable_name(nested_request.get(key))
            if name is not None and name not in values:
                values.append(name)
    return values


def _payload_bool(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.strip().lower() in {"1", "true", "yes", "terminal"}
    return value == 1


def _drop_event_records(
    connection: sqlite3.Connection,
    session_ids: set[str] | None = None,
) -> list[dict[str, Any]]:
    """Read only bounded metadata for context drop events, never their raw payload."""

    records: list[dict[str, Any]] = []
    for event in rows(
        connection,
        "events",
        ("seq", "session_id", "operation_id", "timestamp", "event_type", "payload"),
    ):
        event_type = safe_name(event.get("event_type"))
        if not (event_type.startswith("context.analysis.") and event_type.endswith(".dropped")):
            continue
        payload = _payload_object(event.get("payload"))
        event_session_id = _nullable_name(payload.get("session_id")) or _nullable_name(
            event.get("session_id")
        )
        if session_ids is not None and event_session_id not in session_ids:
            continue
        try:
            dropped_work = max(int(payload.get("dropped_count", 1)), 0)
        except (TypeError, ValueError):
            dropped_work = 1
        reason = _nullable_name(payload.get("reason")) or "unknown"
        terminal = _payload_bool(payload.get("terminal")) or payload.get("outcome") in {"drop", "dropped"}
        request_ids = _payload_ids(
            payload,
            "provider_request_id",
            "request_id",
            "provider_request_ids",
            "request_ids",
        )
        forward_id = _nullable_name(payload.get("forward_id"))
        records.append(
            {
                "event_seq": int_value(event, "seq"),
                "session_id": _nullable_name(payload.get("session_id"))
                or _nullable_name(event.get("session_id")),
                "operation_id": _nullable_name(payload.get("operation_id"))
                or _nullable_name(event.get("operation_id")),
                "timestamp": _nullable_name(event.get("timestamp")),
                "reason": reason,
                "dropped_work": dropped_work,
                "request_ids": request_ids,
                "forward_id": forward_id,
                "terminal": terminal,
                "active_forwards": payload.get("active_forwards"),
                "queue_state": _nullable_name(payload.get("queue_state")),
                "analysis_slot_state": _nullable_name(payload.get("analysis_slot_state")),
            }
        )
    return records


def _request_is_analysis_eligible(request: dict[str, Any]) -> bool:
    request_kind = _nullable_name(request.get("request_kind"))
    if request_kind in ELIGIBLE_REQUEST_KINDS:
        return True
    method = safe_name(request.get("method"), "").lower()
    route = safe_name(request.get("route"), "").strip("/").lower()
    return method == "post" and route in {
        "responses",
        "v1/responses",
        "backend-api/codex/responses",
    }


def request_analysis_ledger(
    requests: list[dict[str, Any]],
    snapshots: list[dict[str, Any]],
    drop_events: list[dict[str, Any]],
    operation_session: dict[str, str],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Build one exclusive analysis outcome for every distinct provider request.

    Durable snapshots are authoritative for request outcomes. Drop events are retained as
    operational evidence and only affect an outcome when they explicitly declare a terminal
    classification. An aggregate event without a request identity is never assigned by
    inference; it makes measurement integrity fail instead.
    """

    ledger_by_request: dict[str, dict[str, Any]] = {}
    conflict_counts: Counter[str] = Counter()
    conflict_requests: dict[str, set[str]] = defaultdict(set)

    def conflict(kind: str, request_id: str | None = None) -> None:
        conflict_counts[kind] += 1
        if request_id is not None:
            conflict_requests[kind].add(request_id)

    for request in requests:
        request_id = _nullable_name(request.get("request_id"))
        if request_id is None:
            conflict("provider_request_without_id")
            continue
        if request_id in ledger_by_request:
            conflict("duplicate_provider_request", request_id)
            continue
        operation_id = _nullable_name(request.get("operation_id"))
        ledger_by_request[request_id] = {
            "session_id": operation_session.get(operation_id or ""),
            "provider_request_id": request_id,
            "forward_id": None,
            "eligible": _request_is_analysis_eligible(request),
            "snapshot_id": None,
            "snapshot_status": None,
            "terminal_snapshot_count": 0,
            "durable_drop_events": 0,
            "auxiliary_drop_work": 0,
            "drop_reasons": [],
            "correlation_status": None,
            "request_kind": _nullable_name(request.get("request_kind")),
            "request_bytes": int_value(request, "request_bytes"),
            "outcome": "Ineligible",
            "outcome_notes": [],
            "observation_status": _nullable_name(request.get("observation_status")),
        }

    snapshots_by_request: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for snapshot in snapshots:
        request_id = _nullable_name(snapshot.get("provider_request_id"))
        if request_id is None or request_id not in ledger_by_request:
            conflict("snapshot_without_provider_request", request_id)
            continue
        if not ledger_by_request[request_id]["eligible"]:
            conflict("snapshot_for_ineligible_request", request_id)
            continue
        snapshots_by_request[request_id].append(snapshot)

    for request_id, record in ledger_by_request.items():
        if not record["eligible"]:
            continue
        request_snapshots = snapshots_by_request.get(request_id, [])
        terminal_snapshots = [
            snapshot
            for snapshot in request_snapshots
            if int_value(snapshot, "completed_at_us") is not None
            or int_value(snapshot, "recovered_at_us") is not None
        ]
        record["terminal_snapshot_count"] = len(terminal_snapshots)
        if len(terminal_snapshots) > 1:
            conflict("multiple_terminal_snapshots", request_id)
        selected = sorted(
            terminal_snapshots or request_snapshots,
            key=lambda snapshot: (
                int_value(snapshot, "completed_at_us") is None,
                int_value(snapshot, "completed_at_us") or 0,
                _nullable_name(snapshot.get("snapshot_id")) or "",
            ),
        )[-1:] or []
        if selected:
            snapshot = selected[0]
            record["snapshot_id"] = _nullable_name(snapshot.get("snapshot_id"))
            record["snapshot_status"] = _nullable_name(snapshot.get("status"))
            record["correlation_status"] = _nullable_name(snapshot.get("correlation_status"))
            if (
                safe_name(snapshot.get("status")) == "complete"
                and int_value(snapshot, "completed_at_us") is None
            ):
                conflict("complete_snapshot_without_completion_timestamp", request_id)
            if terminal_snapshots:
                record["outcome"] = (
                    "Complete" if safe_name(snapshot.get("status")) == "complete" else "Partial"
                )
            else:
                record["outcome"] = "Dropped"
                record["outcome_notes"].append("non_terminal_snapshot")
                conflict("non_terminal_snapshot", request_id)
        else:
            record["outcome"] = "Dropped"
            record["outcome_notes"].append("missing_snapshot")

    auxiliary_drop_events = 0
    auxiliary_drop_work = 0
    unmatched_events = 0
    unmatched_drop_work = 0
    drop_diagnostics: list[dict[str, Any]] = []

    def diagnostic(
        event: dict[str, Any],
        request_id: str | None,
        record: dict[str, Any] | None,
    ) -> None:
        drop_diagnostics.append(
            {
                "event_seq": event.get("event_seq"),
                "provider_request_id": request_id,
                "forward_id": event.get("forward_id") or (record or {}).get("forward_id"),
                "session_id": event.get("session_id") or (record or {}).get("session_id"),
                "request_kind": (record or {}).get("request_kind"),
                "request_bytes": (record or {}).get("request_bytes"),
                "timestamp": event.get("timestamp"),
                "reason": event.get("reason"),
                "active_forwards": event.get("active_forwards"),
                "queue_state": event.get("queue_state"),
                "analysis_slot_state": event.get("analysis_slot_state"),
                "classification": "terminal" if event.get("terminal") else "auxiliary",
                "dropped_work": event.get("dropped_work"),
            }
        )

    for event in drop_events:
        auxiliary_drop_events += 1
        dropped_work = int_value(event, "dropped_work") or 0
        auxiliary_drop_work += dropped_work
        request_ids = list(event.get("request_ids") or [])
        matched_ids = [
            request_id
            for request_id in request_ids
            if request_id in ledger_by_request and ledger_by_request[request_id]["eligible"]
        ]
        if not matched_ids:
            unmatched_events += 1
            unmatched_drop_work += dropped_work
            conflict("drop_without_identifiable_eligible_request")
            diagnostic(event, None, None)
            continue
        if len(matched_ids) != len(request_ids):
            unmatched_events += 1
            unmatched_drop_work += dropped_work
            conflict("drop_references_unknown_or_ineligible_request")
        for request_id in matched_ids:
            record = ledger_by_request[request_id]
            record["durable_drop_events"] += 1
            record["auxiliary_drop_work"] += dropped_work
            reason = event["reason"]
            if reason not in record["drop_reasons"]:
                record["drop_reasons"].append(reason)
            if event.get("forward_id") is not None:
                record["forward_id"] = event["forward_id"]
            if event.get("terminal"):
                if record["outcome"] != "Dropped":
                    conflict("terminal_drop_conflicts_with_snapshot", request_id)
                    record["outcome_notes"].append("terminal_drop_conflicts_with_snapshot")
                else:
                    record["outcome_notes"].append("terminal_drop_confirmed")
            diagnostic(event, request_id, record)

    diagnosed_request_ids = {
        row["provider_request_id"]
        for row in drop_diagnostics
        if row.get("provider_request_id") is not None
    }
    for record in ledger_by_request.values():
        if not record["eligible"] or record["outcome"] != "Dropped":
            continue
        request_id = record["provider_request_id"]
        if request_id in diagnosed_request_ids:
            continue
        drop_diagnostics.append(
            {
                "event_seq": None,
                "provider_request_id": request_id,
                "forward_id": record["forward_id"],
                "session_id": record["session_id"],
                "request_kind": record["request_kind"],
                "request_bytes": record["request_bytes"],
                "timestamp": None,
                "reason": "missing_durable_snapshot",
                "active_forwards": None,
                "queue_state": None,
                "analysis_slot_state": None,
                "classification": "request_outcome",
                "dropped_work": 1,
            }
        )

    complete_ids = {
        request_id
        for request_id, record in ledger_by_request.items()
        if record["eligible"] and record["outcome"] == "Complete"
    }
    partial_ids = {
        request_id
        for request_id, record in ledger_by_request.items()
        if record["eligible"] and record["outcome"] == "Partial"
    }
    dropped_ids = {
        request_id
        for request_id, record in ledger_by_request.items()
        if record["eligible"] and record["outcome"] == "Dropped"
    }
    eligible_ids = {
        request_id for request_id, record in ledger_by_request.items() if record["eligible"]
    }
    outcome_partition_valid = (
        len(eligible_ids) == len(complete_ids) + len(partial_ids) + len(dropped_ids)
        and complete_ids.isdisjoint(partial_ids)
        and complete_ids.isdisjoint(dropped_ids)
        and partial_ids.isdisjoint(dropped_ids)
        and all(
            len(outcome_ids) <= len(eligible_ids)
            for outcome_ids in (complete_ids, partial_ids, dropped_ids)
        )
    )
    if not outcome_partition_valid:
        conflict("invalid_outcome_partition")

    integrity_reasons: list[str] = []
    if conflict_counts:
        integrity_reasons.append("analysis_outcome_conflicts")
    if unmatched_events:
        integrity_reasons.append("unmatched_drop_events")
    if not outcome_partition_valid:
        integrity_reasons.append("invalid_outcome_partition")
    summary = {
        "ledger_rows": len(ledger_by_request),
        "provider_requests": len(ledger_by_request),
        "eligible_requests": len(eligible_ids),
        "complete_requests": len(complete_ids),
        "partial_requests": len(partial_ids),
        "dropped_requests": len(dropped_ids),
        "ineligible_requests": len(ledger_by_request) - len(eligible_ids),
        "auxiliary_drop_events": auxiliary_drop_events,
        "auxiliary_drop_work": auxiliary_drop_work,
        "unmatched_events": unmatched_events,
        "unmatched_drop_work": unmatched_drop_work,
        "request_coverage": ratio(len(complete_ids), len(eligible_ids)),
        "analysis_outcome_conflicts": sum(conflict_counts.values()),
        "conflict_types": dict(conflict_counts),
        "conflict_request_counts": {
            kind: len(request_ids) for kind, request_ids in conflict_requests.items()
        },
        "outcome_partition_valid": outcome_partition_valid,
        "measurement_integrity": "passed" if not integrity_reasons else "failed",
        "integrity_reasons": integrity_reasons,
        "drop_diagnostics": drop_diagnostics,
    }
    return list(ledger_by_request.values()), summary


def _quartile_label(value: int | None, boundaries: tuple[float | None, ...]) -> str:
    if value is None or not boundaries or boundaries[0] is None:
        return "unknown"
    if value <= boundaries[0]:
        return "q1"
    if len(boundaries) > 1 and boundaries[1] is not None and value <= boundaries[1]:
        return "q2"
    if len(boundaries) > 2 and boundaries[2] is not None and value <= boundaries[2]:
        return "q3"
    return "q4"


def _outcome_group(records: Iterable[dict[str, Any]], key: str) -> list[dict[str, Any]]:
    groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        if record.get("eligible"):
            groups[safe_name(record.get(key))].append(record)
    result = []
    for name, group in sorted(groups.items()):
        complete = sum(row.get("outcome") == "Complete" for row in group)
        partial = sum(row.get("outcome") == "Partial" for row in group)
        dropped = sum(row.get("outcome") == "Dropped" for row in group)
        result.append(
            {
                "name": name,
                "eligible_requests": len(group),
                "complete_requests": complete,
                "partial_requests": partial,
                "dropped_requests": dropped,
                "request_coverage": ratio(complete, len(group)),
                "drop_rate": ratio(dropped, len(group)),
            }
        )
    return result


def missingness_report(
    ledger: list[dict[str, Any]],
    snapshot_visible_tokens: dict[str, int],
    cohort_kind: str,
    session_metadata: dict[str, dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Describe whether missing analysis is concentrated in observable request strata."""

    eligible = [row for row in ledger if row.get("eligible")]
    request_sizes = [int_value(row, "request_bytes") for row in eligible]
    request_sizes = [value for value in request_sizes if value is not None]
    request_boundaries = tuple(percentile(request_sizes, value) for value in (25, 50, 75))
    context_sizes = [
        snapshot_visible_tokens.get(safe_name(row.get("snapshot_id")))
        for row in eligible
        if row.get("snapshot_id") is not None
    ]
    context_sizes = [value for value in context_sizes if value is not None]
    context_boundaries = tuple(percentile(context_sizes, value) for value in (25, 50, 75))

    session_counts: Counter[str] = Counter(safe_name(row.get("session_id")) for row in eligible)
    session_indexes: Counter[str] = Counter()
    session_metadata = session_metadata or {}
    for row in ledger:
        if not row.get("eligible"):
            continue
        session_id = safe_name(row.get("session_id"))
        session_indexes[session_id] += 1
        row["turn_index"] = session_indexes[session_id]
        row["session_request_count"] = session_counts[session_id]
        request_size = int_value(row, "request_bytes")
        row["request_size_quartile"] = _quartile_label(request_size, request_boundaries)
        context_size = snapshot_visible_tokens.get(safe_name(row.get("snapshot_id")))
        row["context_size_quartile"] = _quartile_label(context_size, context_boundaries)
        metadata = session_metadata.get(session_id, {})
        row["workload"] = _nullable_name(metadata.get("workload"))
        row["concurrency_mode"] = _nullable_name(metadata.get("concurrency_mode"))

    workload_available = bool(session_metadata)
    concurrency_available = bool(session_metadata)
    unavailable_dimensions = []
    if not workload_available:
        unavailable_dimensions.append("workload")
    if not concurrency_available:
        unavailable_dimensions.append("concurrency_mode")

    return {
        "cohort_kind": cohort_kind,
        "eligible_requests": len(eligible),
        "request_size_boundaries": request_boundaries,
        "context_size_boundaries": context_boundaries,
        "by_request_kind": _outcome_group(eligible, "request_kind"),
        "by_request_size_quartile": _outcome_group(eligible, "request_size_quartile"),
        "by_context_size_quartile": _outcome_group(eligible, "context_size_quartile"),
        "by_turn_index": _outcome_group(eligible, "turn_index"),
        "by_session_request_count": _outcome_group(eligible, "session_request_count"),
        "by_observation_status": _outcome_group(eligible, "observation_status"),
        "by_workload": _outcome_group(eligible, "workload") if workload_available else [],
        "by_concurrency_mode": (
            _outcome_group(eligible, "concurrency_mode") if concurrency_available else []
        ),
        "unavailable_dimensions": unavailable_dimensions,
    }


def scheduler_report(
    runtime_metrics: dict[str, Any] | None,
    analysis_integrity: dict[str, Any],
    sessions_total: int,
) -> dict[str, Any]:
    """Aggregate bounded runtime scheduler evidence from a metadata-only sidecar."""

    raw_sessions = runtime_metrics.get("sessions", []) if isinstance(runtime_metrics, dict) else []
    sessions = [record for record in raw_sessions if isinstance(record, dict)]

    def session_values(*keys: str) -> list[int]:
        values = []
        for session in sessions:
            value = first_int(session, *keys)
            if value is not None:
                values.append(value)
        return values

    def session_total(*keys: str) -> int:
        return sum(session_values(*keys))

    deferred_total = session_total("deferred_total", "analysis_deferred_total")
    admitted_total = session_total("analysis_admitted_total", "admitted_total")
    processed_total = session_total("processed_deferred_total")
    capacity_drops = session_total("backlog_capacity_drops")
    high_water_items = session_values("high_water_items", "deferred_high_water_items")
    high_water_bytes = session_values("high_water_bytes", "deferred_high_water_bytes")
    wait_us = session_values("analysis_wait_us")
    drain_us = session_values("drain_duration_us", "analysis_drain_us")
    eligible = analysis_integrity["eligible_requests"]
    dropped = analysis_integrity["dropped_requests"]

    return {
        "available": bool(sessions),
        "sessions_with_metrics": len(sessions),
        "sessions_missing_metrics": max(sessions_total - len(sessions), 0) if sessions else None,
        "admitted_total": admitted_total if sessions else None,
        "deferred_total": deferred_total if sessions else None,
        "processed_deferred_total": processed_total if sessions else None,
        "backlog_capacity_drops": capacity_drops if sessions else None,
        "analysis_deferral_rate": ratio(deferred_total, eligible) if sessions else None,
        "analysis_loss_rate": ratio(dropped, eligible) if sessions else None,
        "high_water_items": distribution(high_water_items),
        "high_water_bytes": distribution(high_water_bytes),
        "analysis_wait_us": distribution(wait_us),
        "drain_duration_us": distribution(drain_us),
        "session_metadata": [
            {
                key: jsonable(value)
                for key, value in record.items()
                if key
                in {
                    "session_id",
                    "workload",
                    "concurrency_mode",
                    "high_water_items",
                    "high_water_bytes",
                    "analysis_wait_us",
                    "deferred_total",
                    "analysis_deferred_total",
                    "processed_deferred_total",
                    "backlog_capacity_drops",
                    "drain_duration_us",
                    "analysis_drain_us",
                }
            }
            for record in sessions
        ],
        "source": "metadata_sidecar",
        "notes": [
            "deferral is waiting/admission, not analysis loss",
            "loss rate is derived from exclusive request outcomes",
            "high-water values are per-session maxima",
            "runtime metrics contain no request or response payloads",
        ],
        "accounting": {
            "eligible_requests": eligible,
            "dropped_requests": dropped,
            "admitted_total": admitted_total if sessions else None,
        },
    }


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


def repetition_by_group(
    blocks: list[dict[str, Any]],
    snapshot_context: dict[str, dict[str, Any]],
    fields: tuple[str, ...],
) -> list[dict[str, Any]]:
    """Break repetition and persistence down by safe structural category fields."""

    enriched: list[dict[str, Any]] = []
    for block in blocks:
        context = snapshot_context.get(safe_name(block.get("snapshot_id")), {})
        enriched.append(
            {
                **block,
                "_session_id": context.get("session_id", "unknown"),
                "_snapshot_order": context.get("snapshot_order", 0),
                "_group": tuple(safe_name(block.get(field)) for field in fields),
                "_estimated": int_value(block, "estimated_tokens"),
            }
        )

    def fingerprint_groups(field: str) -> dict[tuple[str, bytes], list[dict[str, Any]]]:
        result: dict[tuple[str, bytes], list[dict[str, Any]]] = defaultdict(list)
        for block in enriched:
            fingerprint = fingerprint_value(block.get(field))
            if fingerprint is not None:
                result[(block["_session_id"], fingerprint)].append(block)
        return result

    details: dict[tuple[str, ...], dict[str, Any]] = defaultdict(
        lambda: {
            "block_count": 0,
            "estimated_tokens": 0,
            "exact_repeated_blocks": 0,
            "exact_repeated_estimated_tokens": 0,
            "semantic_repeated_blocks": 0,
            "semantic_repeated_estimated_tokens": 0,
            "persistence_values": [],
            "unique_semantic_information": 0,
        }
    )
    for block in enriched:
        detail = details[block["_group"]]
        detail["block_count"] += 1
        detail["estimated_tokens"] += block["_estimated"] or 0

    for field, prefix in (
        ("exact_fingerprint", "exact"),
        ("semantic_fingerprint", "semantic"),
    ):
        for records in fingerprint_groups(field).values():
            records.sort(key=lambda item: (item["_snapshot_order"], int_value(item, "ordinal") or 0))
            for record in records[1:]:
                detail = details[record["_group"]]
                detail[f"{prefix}_repeated_blocks"] += 1
                detail[f"{prefix}_repeated_estimated_tokens"] += record["_estimated"] or 0
            if field == "semantic_fingerprint":
                first = records[0]
                details[first["_group"]]["unique_semantic_information"] += first["_estimated"] or 0

    persistence_groups: dict[tuple[str, ...], list[int]] = defaultdict(list)
    seen_fingerprints: set[tuple[str, str, bytes]] = set()
    for field in ("semantic_fingerprint", "exact_fingerprint"):
        for _key, records in fingerprint_groups(field).items():
            records.sort(key=lambda item: (item["_snapshot_order"], int_value(item, "ordinal") or 0))
            fingerprint = fingerprint_value(records[0].get(field))
            marker = (field, records[0]["_session_id"], fingerprint or b"")
            if marker in seen_fingerprints:
                continue
            seen_fingerprints.add(marker)
            by_group: dict[tuple[str, ...], set[int]] = defaultdict(set)
            for record in records:
                by_group[record["_group"]].add(record["_snapshot_order"])
            for group, orders in by_group.items():
                persistence_groups[group].append(len(orders))

    result = []
    for group, detail in sorted(details.items(), key=lambda item: (-item[1]["estimated_tokens"], item[0])):
        persistence_values = persistence_groups.get(group, [])
        effective = detail["estimated_tokens"]
        unique = detail["unique_semantic_information"]
        result.append(
            {
                **dict(zip(fields, group)),
                "name": " | ".join(f"{field}={value}" for field, value in zip(fields, group)),
                "block_count": detail["block_count"],
                "estimated_tokens": effective,
                "token_share": None,
                "exact_repeated_blocks": detail["exact_repeated_blocks"],
                "exact_repeated_estimated_tokens": detail["exact_repeated_estimated_tokens"],
                "semantic_repeated_blocks": detail["semantic_repeated_blocks"],
                "semantic_repeated_estimated_tokens": detail["semantic_repeated_estimated_tokens"],
                "unique_semantic_information_estimated_tokens": unique,
                "redundancy_factor": ratio(effective, unique),
                "persistence": distribution(persistence_values),
                "average_persistence": statistics.mean(persistence_values) if persistence_values else None,
            }
        )
    total = sum(row["estimated_tokens"] for row in result)
    for row in result:
        row["token_share"] = ratio(row["estimated_tokens"], total)
    return result


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
    measurement_instrument_version: int = DEFAULT_MEASUREMENT_INSTRUMENT_VERSION,
    runtime_metrics: dict[str, Any] | None = None,
    session_ids: set[str] | None = None,
) -> dict[str, Any]:
    connection.row_factory = sqlite3.Row
    all_session_rows = rows(connection, "sessions", ("session_id", "state", "ended_at"))
    requested_session_ids = sorted(session_ids) if session_ids is not None else None
    missing_session_ids = (
        sorted(
            set(requested_session_ids or ())
            - {safe_name(row.get("session_id")) for row in all_session_rows}
        )
        if requested_session_ids is not None
        else []
    )
    if missing_session_ids:
        raise ValueError(f"requested session IDs are absent from database: {missing_session_ids}")
    selected_session_ids = (
        {safe_name(row.get("session_id")) for row in all_session_rows}
        if session_ids is None
        else set(session_ids)
    )
    session_rows = [
        row for row in all_session_rows if safe_name(row.get("session_id")) in selected_session_ids
    ]
    operation_rows = rows(connection, "operations", ("operation_id", "session_id", "kind", "status"))
    operation_rows = [
        row for row in operation_rows if safe_name(row.get("session_id")) in selected_session_ids
    ]
    operation_ids = {safe_name(row.get("operation_id")) for row in operation_rows}
    request_rows = rows(
        connection,
        "provider_requests",
        (
            "request_id",
            "operation_id",
            "route",
            "method",
            "request_bytes",
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
    request_rows = [row for row in request_rows if safe_name(row.get("operation_id")) in operation_ids]
    request_ids = {safe_name(row.get("request_id")) for row in request_rows}
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
    attempt_rows = [row for row in attempt_rows if safe_name(row.get("request_id")) in request_ids]
    attempt_ids = {safe_name(row.get("attempt_id")) for row in attempt_rows}
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
    usage_rows = [row for row in usage_rows if safe_name(row.get("attempt_id")) in attempt_ids]
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
            "recovered_at_us",
            "correlation_status",
            "explicit_request_complete",
        ),
    )
    snapshot_rows = [
        row
        for row in snapshot_rows
        if safe_name(row.get("session_id")) in selected_session_ids
        and safe_name(row.get("provider_request_id")) in request_ids
    ]
    snapshot_ids = {safe_name(row.get("snapshot_id")) for row in snapshot_rows}
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
    metric_rows = [row for row in metric_rows if safe_name(row.get("snapshot_id")) in snapshot_ids]
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
    block_rows = [row for row in block_rows if safe_name(row.get("snapshot_id")) in snapshot_ids]
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
    delta_rows = [row for row in delta_rows if safe_name(row.get("current_snapshot_id")) in snapshot_ids]
    reconciliation_rows = rows(
        connection,
        "token_reconciliations",
        ("snapshot_id", "visible_estimated_tokens", "provider_input_tokens", "residual_tokens", "comparability"),
    )
    reconciliation_rows = [
        row for row in reconciliation_rows if safe_name(row.get("snapshot_id")) in snapshot_ids
    ]
    event_session_filter = selected_session_ids if session_ids is not None else None
    counts, drop_reasons, drop_work = event_counts(connection, event_session_filter)
    drop_events = _drop_event_records(connection, event_session_filter)
    attempts_by_request, forwarding_errors = request_attempts(request_rows, attempt_rows)

    operation_session = {safe_name(row.get("operation_id")): safe_name(row.get("session_id")) for row in operation_rows}
    request_session = {safe_name(row.get("request_id")): operation_session.get(safe_name(row.get("operation_id")), "unknown") for row in request_rows}
    ledger_rows, analysis_integrity = request_analysis_ledger(
        request_rows,
        snapshot_rows,
        drop_events,
        operation_session,
    )
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
    snapshot_visible_tokens: dict[str, int] = {}
    for snapshot_id, blocks in by_snapshot_blocks.items():
        estimates = [int_value(block, "estimated_tokens") for block in blocks]
        estimates = [value for value in estimates if value is not None]
        if estimates:
            snapshot_visible_tokens[snapshot_id] = sum(estimates)
    block_estimates = [int_value(block, "estimated_tokens") for block in block_rows]
    estimated_block_values = [value for value in block_estimates if value is not None]
    estimated_token_total = sum(estimated_block_values)
    raw_token_total = sum(int_value(block, "raw_bytes") or 0 for block in block_rows)

    analysis_seen = analysis_integrity["eligible_requests"]
    analysis_complete = analysis_integrity["complete_requests"]
    analysis_partial = analysis_integrity["partial_requests"]
    analysis_dropped = analysis_integrity["dropped_requests"]
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
    estimation_coverage = {
        "by_detected_content_kind": estimation_coverage_by(block_rows, "detected_kind"),
        "by_context_block_kind": estimation_coverage_by(block_rows, "kind"),
        "by_context_origin": estimation_coverage_by(block_rows, "origin"),
    }
    cross_composition = aggregate_cross(
        block_rows,
        ("origin", "kind", "detected_kind"),
    )
    composition = {
        "by_context_block_kind": aggregate(block_rows, "kind"),
        "by_role": aggregate(block_rows, "role"),
        "by_origin": aggregate(block_rows, "origin"),
        "detected_content": detected_content,
        "by_origin_kind_detected_content": cross_composition,
        "estimated_token_total": estimated_token_total,
        "raw_bytes_total": raw_token_total,
    }
    repetition, _persistence, category_details = repetition_and_persistence(block_rows, snapshot_context)
    cross_repetition = repetition_by_group(
        block_rows,
        snapshot_context,
        ("origin", "kind", "detected_kind"),
    )
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
    runtime_session_metadata = {
        safe_name(record.get("session_id")): record
        for record in (runtime_metrics or {}).get("sessions", [])
        if isinstance(record, dict) and record.get("session_id") is not None
    }
    missingness = missingness_report(
        ledger_rows,
        snapshot_visible_tokens,
        cohort_kind,
        runtime_session_metadata,
    )
    estimator_coverage = ratio(len(estimated_block_values), len(block_rows))
    semantic_coverage = ratio(sum(semantic_values), len(semantic_values) * 10_000) if semantic_values else None
    ranking = opportunity_ranking(detected_content, category_details, estimator_coverage, semantic_coverage)
    scheduler = scheduler_report(runtime_metrics, analysis_integrity, len(session_rows))
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
            "measurement_instrument_version": measurement_instrument_version,
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
            "session_filter": {
                "applied": requested_session_ids is not None,
                "selected_session_ids": requested_session_ids,
                "excluded_sessions": len(all_session_rows) - len(session_rows),
            },
        },
        "quality": {
            "analysis_requests_seen": analysis_seen,
            "analysis_complete": analysis_complete,
            "analysis_partial": analysis_partial,
            "analysis_dropped": analysis_dropped,
            "analysis_coverage": analysis_integrity["request_coverage"],
            "analysis_auxiliary_drop_events": analysis_integrity["auxiliary_drop_events"],
            "analysis_auxiliary_drop_work": analysis_integrity["auxiliary_drop_work"],
            "analysis_unmatched_events": analysis_integrity["unmatched_events"],
            "analysis_unmatched_drop_work": analysis_integrity["unmatched_drop_work"],
            "analysis_outcome_conflicts": analysis_integrity["analysis_outcome_conflicts"],
            "measurement_integrity": analysis_integrity["measurement_integrity"],
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
                **estimation_coverage,
            },
            "forwarding_errors": forwarding_errors,
            "context_malformed": malformed,
            "provider_usage_final": usage_final_count,
            "provider_observation_partial": counts.get("provider.observation.partial", 0),
            "snapshot_statuses": dict(distinct_statuses),
            "event_drop_reasons": dict(drop_reasons),
            "event_drop_work": dict(drop_work),
            "analysis_deferral_rate": scheduler["analysis_deferral_rate"],
            "analysis_loss_rate": scheduler["analysis_loss_rate"],
            "backlog_capacity_drops": scheduler["backlog_capacity_drops"],
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
        "repetition_by_origin_kind_detected_content": cross_repetition,
        "stable_prefix": stable_prefix,
        "scheduler": scheduler,
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
        "analysis_integrity": analysis_integrity,
        "request_analysis_ledger": ledger_rows,
        "missingness": missingness,
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
        "Metadata-only report. No request/response payloads, raw fingerprints, headers, or prices are included; the ledger contains opaque storage identities for accounting.",
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
        f"- Request ledger: {format_number(report['analysis_integrity']['eligible_requests'])} eligible; {format_number(report['analysis_integrity']['complete_requests'])} complete; {format_number(report['analysis_integrity']['partial_requests'])} partial; {format_number(report['analysis_integrity']['dropped_requests'])} dropped.",
        f"- Measurement integrity: `{report['analysis_integrity']['measurement_integrity']}`; conflicts: {format_number(report['analysis_integrity']['analysis_outcome_conflicts'])}; unmatched drop events: {format_number(report['analysis_integrity']['unmatched_events'])}.",
        f"- Auxiliary drops: {format_number(report['analysis_integrity']['auxiliary_drop_events'])} events / {format_number(report['analysis_integrity']['auxiliary_drop_work'])} work units.",
        f"- Correlation: {format_pct(quality['correlation_coverage'])} ({quality['correlation_correlated']}/{quality['correlation_eligible']}).",
        f"- Forwarding errors: {format_number(quality['forwarding_errors'])}.",
        f"- Context malformed: {format_number(quality['context_malformed'])}.",
        f"- Token-estimation coverage by blocks: {format_pct(quality['token_estimation_coverage']['by_blocks'])}.",
        f"- Semantic coverage mean: {format_pct(quality['semantic_coverage']['mean'])}.",
        f"- Provider observation partial: {format_number(quality['provider_observation_partial'])}.",
        f"- Analysis deferral rate: {format_pct(quality['analysis_deferral_rate'])}.",
        f"- Analysis loss rate: {format_pct(quality['analysis_loss_rate'])}.",
        f"- Backlog capacity drops: {format_number(quality['backlog_capacity_drops'])}.",
        "",
        "## Estimation coverage by category",
        "",
        "The composition token shares use the estimable-token subset; these tables show coverage bias by category.",
        "",
        "| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |",
        "| --- | --- | ---: | ---: | ---: |",
        *[
            f"| detected_content_kind | {row['name']} | {row['estimated_block_count']}/{row['block_count']} ({format_pct(row['block_coverage'])}) | {format_number(row['estimated_raw_bytes'])}/{format_number(row['raw_bytes'])} ({format_pct(row['bytes_coverage'])}) | {format_pct(row['estimated_token_share'])} |"
            for row in quality["token_estimation_coverage"]["by_detected_content_kind"]
        ],
        *[
            f"| context_block_kind | {row['name']} | {row['estimated_block_count']}/{row['block_count']} ({format_pct(row['block_coverage'])}) | {format_number(row['estimated_raw_bytes'])}/{format_number(row['raw_bytes'])} ({format_pct(row['bytes_coverage'])}) | {format_pct(row['estimated_token_share'])} |"
            for row in quality["token_estimation_coverage"]["by_context_block_kind"]
        ],
        *[
            f"| context_origin | {row['name']} | {row['estimated_block_count']}/{row['block_count']} ({format_pct(row['block_coverage'])}) | {format_number(row['estimated_raw_bytes'])}/{format_number(row['raw_bytes'])} ({format_pct(row['bytes_coverage'])}) | {format_pct(row['estimated_token_share'])} |"
            for row in quality["token_estimation_coverage"]["by_context_origin"]
        ],
        "",
        "## Deferred-analysis scheduler",
        "",
        f"- Runtime sidecar available: `{report['scheduler']['available']}`; sessions with metrics: {format_number(report['scheduler']['sessions_with_metrics'])}.",
        f"- Admitted: {format_number(report['scheduler']['admitted_total'])}; deferred: {format_number(report['scheduler']['deferred_total'])}; processed: {format_number(report['scheduler']['processed_deferred_total'])}.",
        f"- High-water items P50/P90/P99: {format_number(report['scheduler']['high_water_items']['p50'])}/{format_number(report['scheduler']['high_water_items']['p90'])}/{format_number(report['scheduler']['high_water_items']['p99'])}.",
        f"- High-water bytes P50/P90/P99: {format_number(report['scheduler']['high_water_bytes']['p50'])}/{format_number(report['scheduler']['high_water_bytes']['p90'])}/{format_number(report['scheduler']['high_water_bytes']['p99'])}.",
        f"- Analysis wait µs P50/P90/P99: {format_number(report['scheduler']['analysis_wait_us']['p50'])}/{format_number(report['scheduler']['analysis_wait_us']['p90'])}/{format_number(report['scheduler']['analysis_wait_us']['p99'])}.",
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
            "## Context composition cross-tab",
            "",
            "| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |",
            "| --- | --- | --- | ---: | ---: | ---: |",
        ]
    )
    for row in report["composition"]["by_origin_kind_detected_content"][:40]:
        lines.append(
            f"| {row['origin']} | {row['kind']} | {row['detected_kind']} | {format_number(row['estimated_tokens'])} | {format_pct(row['token_share'])} | {format_number(row['raw_bytes'])} |"
        )
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
            "## Repetition by structural category",
            "",
            "| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |",
            "| --- | --- | --- | ---: | ---: | ---: | ---: |",
        ]
    )
    for row in report["repetition_by_origin_kind_detected_content"][:40]:
        persistence = row["persistence"]
        lines.append(
            f"| {row['origin']} | {row['kind']} | {row['detected_kind']} | {format_pct(row['token_share'])} | {format_number(row['exact_repeated_estimated_tokens'])} | {format_number(row['semantic_repeated_estimated_tokens'])} | {format_number(persistence['p50'])}/{format_number(persistence['p90'])} |"
        )
    lines.extend(
        [
            "",
            "## Missingness",
            "",
            "Drop rate is based on exclusive request outcomes, never on event count.",
            f"- Request-size quartile boundaries: `{json.dumps(report['missingness']['request_size_boundaries'])}`.",
            f"- Context-size quartile boundaries: `{json.dumps(report['missingness']['context_size_boundaries'])}`.",
            f"- Unavailable dimensions: `{json.dumps(report['missingness']['unavailable_dimensions'])}`.",
            f"- Workload strata: `{json.dumps(report['missingness']['by_workload'], sort_keys=True)}`.",
            f"- Concurrency strata: `{json.dumps(report['missingness']['by_concurrency_mode'], sort_keys=True)}`.",
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
    if report["analysis_integrity"]["drop_diagnostics"]:
        lines.extend(
            [
                "## Drop diagnostics",
                "",
                "Metadata-only diagnostics; unavailable request/forward identities remain `unknown`.",
                "",
                "| Event | Provider request | Forward | Session | Kind | Bytes | Reason | Classification | Work |",
                "| ---: | --- | --- | --- | --- | ---: | --- | --- | ---: |",
            ]
        )
        for row in report["analysis_integrity"]["drop_diagnostics"]:
            lines.append(
                f"| {format_number(row['event_seq'])} | {row['provider_request_id'] or 'unknown'} | {row['forward_id'] or 'unknown'} | {row['session_id'] or 'unknown'} | {row['request_kind'] or 'unknown'} | {format_number(row['request_bytes'])} | {row['reason']} | {row['classification']} | {format_number(row['dropped_work'])} |"
            )
        lines.append("")
    lines.extend(
        [
            "## Missingness strata",
            "",
            "| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for row in report["missingness"]["by_request_size_quartile"]:
        lines.append(
            f"| {row['name']} | {row['eligible_requests']} | {row['complete_requests']} | {row['partial_requests']} | {row['dropped_requests']} | {format_pct(row['request_coverage'])} | {format_pct(row['drop_rate'])} |"
        )
    lines.append("")
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


def read_runtime_metrics(path: Path | None) -> dict[str, Any] | None:
    """Read a metadata-only scheduler sidecar without accepting payload-bearing records."""

    if path is None:
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid runtime metrics sidecar: {path}: {error}") from error
    if not isinstance(value, dict) or not isinstance(value.get("sessions"), list):
        raise ValueError("runtime metrics sidecar must contain a sessions list")
    return value


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, default=default_database())
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--measurement-id", default=DEFAULT_MEASUREMENT_ID)
    parser.add_argument("--cohort-label", required=True, help="Report label, for example n20 or n30")
    parser.add_argument("--cohort-kind", choices=COHORT_KINDS, default=DEFAULT_COHORT_KIND)
    parser.add_argument("--tracepress-commit", default=os.environ.get("TRACEPRESS_COMMIT", "unknown"))
    parser.add_argument("--codex-version", default=os.environ.get("CODEX_VERSION", "unknown"))
    parser.add_argument(
        "--runtime-metrics",
        type=Path,
        help="metadata-only scheduler metrics JSON sidecar captured per session",
    )
    parser.add_argument(
        "--include-session-id",
        action="append",
        dest="include_session_ids",
        help="restrict this report to one database session; repeat for a cohort subset",
    )
    parser.add_argument(
        "--measurement-instrument-version",
        type=int,
        default=int(
            os.environ.get(
                "TRACEPRESS_MEASUREMENT_INSTRUMENT_VERSION",
                str(DEFAULT_MEASUREMENT_INSTRUMENT_VERSION),
            )
        ),
    )
    return parser


def main(arguments: list[str] | None = None) -> int:
    parser = build_parser()
    options = parser.parse_args(arguments)
    if not options.database.is_file():
        parser.error(f"database does not exist: {options.database}")
    try:
        runtime_metrics = read_runtime_metrics(options.runtime_metrics)
        database_uri = f"file:{options.database.resolve()}?mode=ro"
        connection = sqlite3.connect(database_uri, uri=True)
        report = analyze_connection(
            connection,
            measurement_id=options.measurement_id,
            cohort_label=options.cohort_label,
            cohort_kind=options.cohort_kind,
            tracepress_commit=options.tracepress_commit,
            codex_version=options.codex_version,
            measurement_instrument_version=options.measurement_instrument_version,
            runtime_metrics=runtime_metrics,
            session_ids=set(options.include_session_ids) if options.include_session_ids else None,
        )
    except (sqlite3.DatabaseError, ValueError) as error:
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
        f"forwarding_errors={report['quality']['forwarding_errors']} "
        f"measurement_integrity={report['analysis_integrity']['measurement_integrity']}"
    )
    return 0 if report["analysis_integrity"]["measurement_integrity"] == "passed" else 2


if __name__ == "__main__":
    raise SystemExit(main())
