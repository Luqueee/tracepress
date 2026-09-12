#!/usr/bin/env python3
"""Capture one metadata-only scheduler sidecar for one Tracepress run.

The collector never persists child stdout/stderr or command arguments. It consumes only the
allowlisted machine-readable records emitted by ``tracepress run`` and writes one uniquely named
JSON document after the child exits. The document is atomically published and remains compatible
with the ``runtime_metrics.json`` shape consumed by ``analyze_baseline.py``.
"""

from __future__ import annotations

import argparse
from datetime import UTC, datetime
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import time
import uuid
from typing import Any, Sequence


MEASUREMENT_METADATA_PREFIX = b"TRACEPRESS_MEASUREMENT_METADATA="
SCHEDULER_METRICS_PREFIX = b"TRACEPRESS_SCHEDULER_METRICS="
SCHEDULER_FIELDS = (
    "deferred_queue_items",
    "deferred_queue_bytes",
    "deferred_high_water_items",
    "deferred_high_water_bytes",
    "analysis_admitted_total",
    "analysis_deferred_total",
    "processed_deferred_total",
    "backlog_capacity_drops",
    "analysis_wait_us",
)
CONTEXT_COUNTER_FIELDS = (
    "analysis_requests_seen",
    "analysis_requests_complete",
    "analysis_requests_partial",
    "analysis_requests_dropped",
    "correlation_eligible",
    "correlation_correlated",
)
MEASUREMENT_INSTRUMENT_VERSION = 2
DEFAULT_CAPTURE_TIMEOUT_SECONDS = 900.0
MAX_CAPTURE_LINE_BYTES = 64 * 1024


class CaptureProtocolError(ValueError):
    """Raised when the child exceeds the bounded machine-output protocol."""


def utc_now() -> str:
    return datetime.now(UTC).isoformat()


def _parse_machine_line(line: bytes, prefix: bytes) -> dict[str, Any] | None:
    if not line.startswith(prefix):
        return None
    try:
        value = json.loads(line[len(prefix) :].decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def _parse_counter_line(line: bytes) -> tuple[str, int] | None:
    for field in CONTEXT_COUNTER_FIELDS:
        prefix = f"{field}=".encode("ascii")
        if not line.startswith(prefix):
            continue
        try:
            value = int(line[len(prefix) :].decode("ascii"))
        except (UnicodeDecodeError, ValueError):
            return field, -1
        return field, value
    return None


def _iter_capture_lines(stream: Any, timeout_seconds: float):
    selector = selectors.DefaultSelector()
    selector.register(stream, selectors.EVENT_READ)
    buffer = bytearray()
    deadline = time.monotonic() + timeout_seconds
    try:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError
            if not selector.select(remaining):
                raise TimeoutError
            chunk = os.read(stream.fileno(), 8192)
            if not chunk:
                if buffer:
                    if len(buffer) > MAX_CAPTURE_LINE_BYTES:
                        raise CaptureProtocolError("capture_line_too_large")
                    yield bytes(buffer)
                return
            buffer.extend(chunk)
            while True:
                try:
                    newline = buffer.index(10)
                except ValueError:
                    if len(buffer) > MAX_CAPTURE_LINE_BYTES:
                        raise CaptureProtocolError("capture_line_too_large")
                    break
                line = bytes(buffer[:newline])
                del buffer[: newline + 1]
                if len(line) > MAX_CAPTURE_LINE_BYTES:
                    raise CaptureProtocolError("capture_line_too_large")
                yield line
    finally:
        selector.close()


def write_atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".{path.name}.{os.getpid()}.tmp"
    encoded = json.dumps(value, indent=2, sort_keys=True) + "\n"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    fd = os.open(temporary, flags, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(encoded)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    except BaseException:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
        raise


def _terminate_process(process: subprocess.Popen[bytes]) -> None:
    try:
        process.kill()
    except ProcessLookupError:
        pass
    process.wait()


def _capture_document(
    run_id: str,
    process: subprocess.Popen[bytes] | None,
    collector_started_at: str,
    finished_at: str,
    metadata_records: list[dict[str, Any]],
    metrics_records: list[dict[str, Any]],
    counter_values: dict[str, int],
    counter_duplicates: set[str],
    process_start_error: str | None = None,
    capture_error: str | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    reasons: list[str] = []
    metadata = metadata_records[0] if len(metadata_records) == 1 else {}
    metrics = metrics_records[0] if len(metrics_records) == 1 else {}
    if process_start_error is not None:
        reasons.append("process_start_failed")
    if capture_error is not None:
        reasons.append(capture_error)
    if not metadata_records:
        reasons.append("missing_measurement_metadata")
    elif len(metadata_records) > 1:
        reasons.append("duplicate_measurement_metadata")
    if metadata.get("measurement_run_id") != run_id:
        reasons.append("measurement_run_id_mismatch")
    session_id = metadata.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        session_id = None
        reasons.append("missing_session_id")
    tracepress_pid = metadata.get("tracepress_pid")
    if process is not None and tracepress_pid != process.pid:
        reasons.append("tracepress_pid_mismatch")
    if not metrics_records:
        reasons.append("missing_scheduler_metrics")
    elif len(metrics_records) > 1:
        reasons.append("duplicate_scheduler_metrics")
    missing_fields = [field for field in SCHEDULER_FIELDS if not isinstance(metrics.get(field), int)]
    if missing_fields:
        reasons.append("missing_scheduler_fields:" + ",".join(missing_fields))
    exit_status = process.returncode if process is not None else None
    if exit_status != 0:
        reasons.append("process_exit_nonzero")
    if metrics.get("deferred_queue_items") != 0 or metrics.get("deferred_queue_bytes") != 0:
        reasons.append("queue_not_drained")
    missing_counters = [field for field in CONTEXT_COUNTER_FIELDS if field not in counter_values]
    if missing_counters:
        reasons.append("missing_context_counters:" + ",".join(missing_counters))
    if any(value < 0 for value in counter_values.values()):
        reasons.append("invalid_context_counter")
    if counter_duplicates:
        reasons.append("duplicate_context_counters:" + ",".join(sorted(counter_duplicates)))
    capture_complete = not reasons
    row = {
        "measurement_run_id": run_id,
        "session_id": session_id,
        "tracepress_pid": tracepress_pid if isinstance(tracepress_pid, int) else None,
        "started_at": metadata.get("started_at", collector_started_at),
        "finished_at": finished_at,
        "exit_status": exit_status,
        "capture_complete": capture_complete,
        "capture_reasons": reasons,
        **{field: metrics.get(field) for field in SCHEDULER_FIELDS},
        **{field: counter_values.get(field) for field in CONTEXT_COUNTER_FIELDS},
    }
    document = {
        "measurement_instrument_version": MEASUREMENT_INSTRUMENT_VERSION,
        "measurement_run_id": run_id,
        "session_id": session_id,
        "tracepress_pid": row["tracepress_pid"],
        "started_at": row["started_at"],
        "finished_at": finished_at,
        "exit_status": exit_status,
        "capture_complete": capture_complete,
        "capture_reasons": reasons,
        "sessions": [row],
    }
    return document, row


def collect_scheduler_metrics(
    command: Sequence[str],
    output_dir: Path,
    extra_env: dict[str, str] | None = None,
    timeout_seconds: float = DEFAULT_CAPTURE_TIMEOUT_SECONDS,
) -> dict[str, Any]:
    """Run one command and atomically persist its identity-bound scheduler sidecar."""

    if not command:
        raise ValueError("collector command must not be empty")
    if timeout_seconds <= 0:
        raise ValueError("collector timeout must be positive")
    run_id = str(uuid.uuid4())
    output_dir.mkdir(parents=True, exist_ok=True)
    sidecar_path = output_dir / f"{run_id}.json"
    environment = os.environ.copy()
    if extra_env:
        environment.update(extra_env)
    environment["TRACEPRESS_MEASUREMENT_RUN_ID"] = run_id
    collector_started_at = utc_now()
    metadata_records: list[dict[str, Any]] = []
    metrics_records: list[dict[str, Any]] = []
    counter_values: dict[str, int] = {}
    counter_duplicates: set[str] = set()
    process: subprocess.Popen[bytes] | None = None
    process_start_error: str | None = None
    capture_error: str | None = None
    try:
        process = subprocess.Popen(
            list(command),
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        assert process.stdout is not None
        for raw_line in _iter_capture_lines(process.stdout, timeout_seconds):
            line = raw_line.rstrip(b"\r")
            metadata = _parse_machine_line(line, MEASUREMENT_METADATA_PREFIX)
            if metadata is not None:
                metadata_records.append(metadata)
                continue
            metrics = _parse_machine_line(line, SCHEDULER_METRICS_PREFIX)
            if metrics is not None:
                metrics_records.append(metrics)
                continue
            counter = _parse_counter_line(line)
            if counter is not None:
                if counter[0] in counter_values:
                    counter_duplicates.add(counter[0])
                counter_values[counter[0]] = counter[1]
        process.stdout.close()
        process.wait()
    except TimeoutError:
        capture_error = "capture_timeout"
        if process is not None:
            _terminate_process(process)
    except CaptureProtocolError as error:
        capture_error = str(error)
        if process is not None:
            _terminate_process(process)
    except OSError as error:
        if process is None:
            process_start_error = str(error)
        else:
            capture_error = "capture_stream_error"
            _terminate_process(process)
    if process is not None:
        if process.stdout is not None:
            process.stdout.close()
        if process.returncode is None:
            process.wait()
    finished_at = utc_now()
    document, row = _capture_document(
        run_id,
        process,
        collector_started_at,
        finished_at,
        metadata_records,
        metrics_records,
        counter_values,
        counter_duplicates,
        process_start_error,
        capture_error,
    )
    row["sidecar_filename"] = sidecar_path.name
    document["sidecar_filename"] = sidecar_path.name
    document["sessions"][0]["sidecar_filename"] = sidecar_path.name
    write_atomic_json(sidecar_path, document)
    return {**row, "sidecar_path": sidecar_path}


def merge_sidecars(paths: Sequence[Path]) -> dict[str, Any]:
    """Merge individual run sidecars by embedded identity, rejecting collisions."""

    sessions: list[dict[str, Any]] = []
    run_ids: set[str] = set()
    session_ids: set[str] = set()
    for path in paths:
        document = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(document, dict) or not isinstance(document.get("sessions"), list):
            raise ValueError(f"invalid sidecar document: {path}")
        if document.get("measurement_instrument_version") != MEASUREMENT_INSTRUMENT_VERSION:
            raise ValueError(f"unsupported sidecar instrument version: {path}")
        if len(document["sessions"]) != 1:
            raise ValueError(f"sidecar must contain exactly one session row: {path}")
        for row in document["sessions"]:
            if not isinstance(row, dict):
                raise ValueError(f"invalid sidecar session row: {path}")
            for field in (
                "measurement_run_id",
                "session_id",
                "tracepress_pid",
                "started_at",
                "finished_at",
                "exit_status",
                "capture_complete",
                "capture_reasons",
                "sidecar_filename",
            ):
                if document.get(field) != row.get(field):
                    raise ValueError(f"sidecar root/row identity mismatch for {field}: {path}")
            run_id = row.get("measurement_run_id")
            session_id = row.get("session_id")
            if not isinstance(run_id, str) or run_id in run_ids:
                raise ValueError(f"duplicate or missing measurement_run_id: {path}")
            if not isinstance(session_id, str) or session_id in session_ids:
                raise ValueError(f"duplicate or missing session_id: {path}")
            if row.get("capture_complete") is not True:
                raise ValueError(f"sidecar capture is incomplete: {path}")
            run_ids.add(run_id)
            session_ids.add(session_id)
            sessions.append(row)
    return {
        "measurement_instrument_version": MEASUREMENT_INSTRUMENT_VERSION,
        "sessions": sorted(sessions, key=lambda row: row["measurement_run_id"]),
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=DEFAULT_CAPTURE_TIMEOUT_SECONDS,
        help="bounded child-output capture timeout (default: 900 seconds)",
    )
    parser.add_argument("command", nargs=argparse.REMAINDER, help="command to run after `--`")
    return parser


def main(arguments: list[str] | None = None) -> int:
    options = build_parser().parse_args(arguments)
    command = list(options.command)
    if command and command[0] == "--":
        command.pop(0)
    try:
        result = collect_scheduler_metrics(command, options.output_dir, timeout_seconds=options.timeout_seconds)
    except (OSError, ValueError) as error:
        print(f"sidecar collection failed: {error}", file=sys.stderr)
        return 1
    print(f"sidecar_path={result['sidecar_path']}")
    print(f"measurement_run_id={result['measurement_run_id']}")
    print(f"session_id={result['session_id'] or 'unknown'}")
    print(f"capture_complete={str(result['capture_complete']).lower()}")
    print(f"exit_status={result['exit_status']}")
    return 0 if result["capture_complete"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
