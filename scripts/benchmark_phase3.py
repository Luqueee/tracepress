#!/usr/bin/env python3
"""Measure a frozen baseline checkout versus current context analysis OFF and ON.

The harness deliberately uses only the Python standard library. It builds the exact
selected baseline tag in a temporary git worktree and a separate Cargo target directory,
then builds the current checkout with the same release profile. Every arm drives the real
``tracepress run`` path against the same deterministic local upstream, with fresh process and
home isolation per workload/arm. The current binary runs twice with context analysis OFF and
once with it ON, while the baseline is repeated twice. The alternating A/A arms measure
run-to-run noise for both baseline and current OFF before the ON-versus-OFF decision. A fixed
post-response settle gap is applied equally to every arm so detached Phase 3 work from one sample
cannot contaminate the next sample's forwarding timing. The standalone proxy control is
intentionally absent because it does not exercise the production recorder/analysis path.

The command prints a concise comparison table followed by JSON. ``--json-output`` writes the
same JSON document separately for machine use. No request or response content is included in
reports; durable lifecycle counters are included as bounded observability evidence. The
``--self-test`` mode exercises the lifecycle gate without building or starting any process.
"""

from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from statistics import median
from typing import Any, Iterable


BASELINE_TAG = "phase-2-complete"
EXPECTED_BASELINE_COMMIT = "2b5fd6f"
DEFAULT_SAMPLES = 15
DEFAULT_WARMUP = 3
DEFAULT_BURST_WIDTHS = (32, 64, 72)
REQUEST_LIMIT_BYTES = 8 * 1024 * 1024


class BenchmarkError(RuntimeError):
    """A reproducible benchmark setup or measurement failure."""


class Upstream:
    """Small deterministic HTTP/1.1 server with monotonic wire timestamps."""

    def __init__(self, specs: dict[str, dict[str, Any]]) -> None:
        self.specs = specs
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen(256)
        self._listener.settimeout(0.1)
        self.address = self._listener.getsockname()
        self._stop = threading.Event()
        self._lock = threading.Lock()
        self._metrics: dict[str, dict[str, int | str | None]] = {}
        self._threads: list[threading.Thread] = []
        self._thread = threading.Thread(target=self._serve, name="phase3-bench-upstream", daemon=True)
        self._thread.start()

    @property
    def url(self) -> str:
        host, port = self.address
        return f"http://{host}:{port}/v1/responses"

    def _serve(self) -> None:
        while not self._stop.is_set():
            try:
                connection, _address = self._listener.accept()
            except socket.timeout:
                continue
            except OSError:
                if self._stop.is_set():
                    return
                raise
            thread = threading.Thread(
                target=self._handle,
                args=(connection,),
                name="phase3-bench-upstream-connection",
                daemon=True,
            )
            self._threads.append(thread)
            thread.start()

    @staticmethod
    def _read_request(connection: socket.socket) -> tuple[int | None, dict[str, str], bytes]:
        received = bytearray()
        first_byte_ns: int | None = None
        while b"\r\n\r\n" not in received:
            chunk = connection.recv(64 * 1024)
            if not chunk:
                return first_byte_ns, {}, b""
            if first_byte_ns is None:
                first_byte_ns = time.perf_counter_ns()
            received.extend(chunk)
            if len(received) > 128 * 1024:
                raise BenchmarkError("upstream request headers exceeded the benchmark bound")
        header_end = received.index(b"\r\n\r\n") + 4
        header_bytes = bytes(received[:header_end])
        body = bytearray(received[header_end:])
        lines = header_bytes.split(b"\r\n")
        headers: dict[str, str] = {}
        for raw_line in lines[1:]:
            if not raw_line:
                continue
            name, separator, value = raw_line.partition(b":")
            if separator:
                headers[name.decode("latin1").strip().lower()] = value.decode("latin1").strip()
        try:
            content_length = int(headers.get("content-length", "0"))
        except ValueError as error:
            raise BenchmarkError("upstream received an invalid content length") from error
        if content_length < 0 or content_length > REQUEST_LIMIT_BYTES:
            raise BenchmarkError("upstream content length exceeded the benchmark bound")
        while len(body) < content_length:
            chunk = connection.recv(min(64 * 1024, content_length - len(body)))
            if not chunk:
                raise BenchmarkError("upstream received an incomplete request body")
            body.extend(chunk)
        return first_byte_ns, headers, bytes(body[:content_length])

    def _handle(self, connection: socket.socket) -> None:
        connection.settimeout(30.0)
        try:
            request_first_ns, headers, _body = self._read_request(connection)
            if request_first_ns is None:
                return
            request_id = headers.get("x-bench-id")
            scenario = headers.get("x-bench-scenario", "small_json")
            spec = self.specs.get(scenario)
            if request_id is None or spec is None:
                return
            initial_delay = float(spec["initial_delay_s"])
            chunk_delay = float(spec["chunk_delay_s"])
            chunks = [bytes(chunk) for chunk in spec["response_chunks"]]
            content_type = str(spec["content_type"])
            semantic_chunk = int(spec.get("semantic_chunk", -1))
            if initial_delay:
                time.sleep(initial_delay)
            header = (
                b"HTTP/1.1 200 OK\r\n"
                + f"Content-Type: {content_type}\r\n".encode("ascii")
                + f"Content-Length: {sum(map(len, chunks))}\r\n".encode("ascii")
                + b"Connection: close\r\n\r\n"
            )
            connection.sendall(header)
            response_first_ns: int | None = None
            response_semantic_ns: int | None = None
            for index, chunk in enumerate(chunks):
                if response_first_ns is None and chunk:
                    response_first_ns = time.perf_counter_ns()
                if index == semantic_chunk and chunk:
                    response_semantic_ns = time.perf_counter_ns()
                connection.sendall(chunk)
                if index + 1 < len(chunks) and chunk_delay:
                    time.sleep(chunk_delay)
            with self._lock:
                self._metrics[request_id] = {
                    "request_first_ns": request_first_ns,
                    "response_first_ns": response_first_ns,
                    "response_semantic_ns": response_semantic_ns,
                    "scenario": scenario,
                }
        except (BenchmarkError, OSError, socket.timeout):
            # The client-side result records an error without retaining the body or URL.
            return
        finally:
            try:
                connection.close()
            except OSError:
                pass

    def metric(self, request_id: str) -> dict[str, int | str | None] | None:
        with self._lock:
            value = self._metrics.get(request_id)
            return None if value is None else dict(value)

    def wait_for(self, request_ids: Iterable[str], timeout_s: float) -> None:
        wanted = tuple(request_ids)
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if all(self.metric(request_id) is not None for request_id in wanted):
                return
            time.sleep(0.005)
        missing = [request_id for request_id in wanted if self.metric(request_id) is None]
        raise BenchmarkError(f"upstream did not finish {len(missing)} measured requests")

    def close(self) -> None:
        self._stop.set()
        try:
            self._listener.close()
        except OSError:
            pass
        self._thread.join(timeout=2.0)


# This agent is intentionally content-blind in its output.  Bodies are read from a temporary
# file, and only stable ids, status, and monotonic timestamps cross the process boundary.
AGENT_SCRIPT = r'''
import concurrent.futures
import json
import os
import time
import urllib.request

url = os.environ["TRACEPRESS_RESPONSES_URL"]
body = open(os.environ["BENCH_REQUEST_FILE"], "rb").read()
scenario = os.environ["BENCH_SCENARIO"]
tag = os.environ["BENCH_RUN_TAG"]
warmup = int(os.environ["BENCH_WARMUP"])
samples = int(os.environ["BENCH_SAMPLES"])
burst = int(os.environ["BENCH_BURST"])
streaming = os.environ["BENCH_STREAMING"] == "1"
settle_seconds = float(os.environ.get("BENCH_POST_RESPONSE_SETTLE_SECONDS", "0"))
idle_seconds = float(os.environ.get("BENCH_IDLE_SECONDS", "0"))

def one(identifier):
    started_ns = time.perf_counter_ns()
    result = {"id": identifier, "measured": identifier.startswith("sample:"), "start_ns": started_ns}
    request = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Bench-Id": identifier,
            "X-Bench-Scenario": scenario,
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=90) as response:
            result["status"] = response.status
            result["content_type"] = response.headers.get("content-type", "")
            first_ns = None
            semantic_ns = None
            window = bytearray()
            while True:
                chunk = response.read(1)
                if not chunk:
                    break
                now_ns = time.perf_counter_ns()
                if first_ns is None:
                    first_ns = now_ns
                if streaming and semantic_ns is None:
                    window.extend(chunk)
                    if b"event: response.output_text.delta" in window:
                        semantic_ns = now_ns
                    if len(window) > 4096:
                        del window[:-1024]
            result["first_ns"] = first_ns
            result["semantic_ns"] = semantic_ns
            result["end_ns"] = time.perf_counter_ns()
    except Exception as error:
        result["error"] = type(error).__name__
        result["end_ns"] = time.perf_counter_ns()
    if settle_seconds > 0:
        time.sleep(settle_seconds)
    return result

def emit_batch(prefix, count, parallel):
    identifiers = [f"{prefix}:{tag}:{scenario}:{index}" for index in range(count)]
    if parallel:
        with concurrent.futures.ThreadPoolExecutor(max_workers=count) as pool:
            results = list(pool.map(one, identifiers))
    else:
        results = [one(identifier) for identifier in identifiers]
    for result in results:
        print(json.dumps(result, separators=(",", ":")), flush=True)
if idle_seconds > 0:
    time.sleep(idle_seconds)

if scenario.startswith("burst_"):
    for round_index in range(warmup):
        emit_batch(f"warmup:{round_index}", burst, True)
    for round_index in range(samples):
        emit_batch(f"sample:{round_index}", burst, True)
else:
    for index in range(warmup):
        emit_batch(f"warmup:{index}", 1, False)
    for index in range(samples):
        emit_batch(f"sample:{index}", 1, False)
'''


def response_chunks() -> tuple[bytes, bytes, bytes]:
    return (
        b'event: response.created\ndata: {"response":{"id":"bench","status":"in_progress"}}\n\n',
        b'event: response.output_text.delta\ndata: {"delta":"ok"}\n\n',
        b'event: response.completed\ndata: {"response":{"id":"bench","status":"completed","usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}}}\n\n',
    )


def workloads(burst_widths: Iterable[int]) -> tuple[dict[str, dict[str, Any]], dict[str, bytes]]:
    widths = tuple(sorted(set(int(width) for width in burst_widths)))
    if not widths or any(width < 1 for width in widths):
        raise BenchmarkError("burst widths must be positive")
    streamed = response_chunks()
    small_body = json.dumps(
        {"model": "bench", "input": "small", "stream": False}, separators=(",", ":")
    ).encode()
    streamed_body = json.dumps(
        {
            "model": "bench",
            "stream": True,
            "instructions": "Answer briefly and deterministically.",
            "input": [
                {
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Summarize this benchmark."}],
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "lookup_weather",
                    "description": "Return deterministic weather data.",
                    "parameters": {"type": "object", "properties": {"city": {"type": "string"}}},
                }
            ],
        },
        separators=(",", ":"),
    ).encode()
    context_text = "context-block-" * 9_000
    context_body = json.dumps(
        {
            "model": "bench",
            "stream": False,
            "input": [
                {
                    "role": "user",
                    "content": [{"type": "input_text", "text": context_text}],
                }
                for _ in range(8)
            ],
        },
        separators=(",", ":"),
    ).encode()
    over_budget_text = "analysis-limit-block-" * 250_000
    over_budget_body = json.dumps(
        {
            "model": "bench",
            "stream": False,
            "input": [{"role": "user", "content": [{"type": "input_text", "text": over_budget_text}]}],
        },
        separators=(",", ":"),
    ).encode()
    burst_body = json.dumps(
        {
            "model": "bench",
            "stream": False,
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "burst"}]}],
        },
        separators=(",", ":"),
    ).encode()
    bodies = {
        "small_json": small_body,
        "streamed_responses": streamed_body,
        "context_1m": context_body,
        "analysis_over_budget": over_budget_body,
    }
    for width in widths:
        bodies[f"burst_{width}"] = burst_body
    specs: dict[str, dict[str, Any]] = {
        "small_json": {
            "content_type": "application/json",
            "response_chunks": [b'{"id":"bench","status":"completed","output":[]}'],
            "semantic_chunk": -1,
            "initial_delay_s": 0.003,
            "chunk_delay_s": 0.0,
        },
        "streamed_responses": {
            "content_type": "text/event-stream",
            "response_chunks": list(streamed),
            "semantic_chunk": 1,
            "initial_delay_s": 0.003,
            "chunk_delay_s": 0.003,
        },
        "context_1m": {
            "content_type": "application/json",
            "response_chunks": [b'{"id":"bench","status":"completed","output":[]}'],
            "semantic_chunk": -1,
            "initial_delay_s": 0.004,
            "chunk_delay_s": 0.0,
        },
        "analysis_over_budget": {
            "content_type": "application/json",
            "response_chunks": [b'{"id":"bench","status":"completed","output":[]}'],
            "semantic_chunk": -1,
            "initial_delay_s": 0.004,
            "chunk_delay_s": 0.0,
        },
    }
    burst_spec = {
        "content_type": "application/json",
        "response_chunks": [b'{"id":"bench","status":"completed","output":[]}'],
        "semantic_chunk": -1,
        "initial_delay_s": 0.040,
        "chunk_delay_s": 0.0,
    }
    for width in widths:
        specs[f"burst_{width}"] = {**burst_spec, "burst_width": width}
    return specs, bodies


def run_checked(command: list[str], *, cwd: Path, env: dict[str, str], timeout_s: float = 600.0) -> str:
    completed = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout_s,
        check=False,
    )
    if completed.returncode != 0:
        executable = Path(command[0]).name
        detail = (completed.stderr or completed.stdout).strip()
        suffix = f": {detail[-512:]}" if detail else ""
        raise BenchmarkError(f"{executable} failed with status {completed.returncode}{suffix}")
    return completed.stdout


def git_output(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise BenchmarkError(f"git command failed with status {result.returncode}")
    return result.stdout.strip()


def build_checkout(repo: Path, target_dir: Path) -> tuple[Path, Path]:
    target_dir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target_dir)
    command = [
        "cargo",
        "build",
        "--release",
        "--locked",
        "--manifest-path",
        str(repo / "Cargo.toml"),
        "-p",
        "tracepress-cli",
        "-p",
        "tracepress-daemon",
        "--bin",
        "tracepress",
        "--bin",
        "tracepressd",
    ]
    run_checked(command, cwd=repo, env=env, timeout_s=1_800.0)
    return target_dir / "release" / "tracepress", target_dir / "release" / "tracepressd"


def read_rss_kib(pid: int) -> int | None:
    try:
        status = Path(f"/proc/{pid}/status").read_text(encoding="utf-8")
    except OSError:
        return None
    for line in status.splitlines():
        if line.startswith("VmRSS:"):
            fields = line.split()
            if len(fields) >= 2:
                try:
                    return int(fields[1])
                except ValueError:
                    return None
    return None


def monitor_rss(process: subprocess.Popen[str], stop: threading.Event, samples: list[int]) -> None:
    while not stop.is_set():
        value = read_rss_kib(process.pid)
        if value is not None:
            samples.append(value)
        stop.wait(0.005)
    value = read_rss_kib(process.pid)
    if value is not None:
        samples.append(value)


def percentile95(values: list[float | int]) -> float | int:
    if not values:
        raise BenchmarkError("cannot compute a percentile with no samples")
    ordered = sorted(values)
    rank = max(1, math.ceil(0.95 * len(ordered))) - 1
    return ordered[rank]


def summarize(values: list[float | int]) -> dict[str, float | int | None]:
    if not values:
        return {"median": None, "p95": None, "sample_count": 0}
    return {
        "median": round(float(median(values)), 3),
        "p95": round(float(percentile95(values)), 3),
        "sample_count": len(values),
    }


def parse_agent_output(stdout: str) -> tuple[list[dict[str, Any]], dict[str, int]]:
    records: list[dict[str, Any]] = []
    counters: dict[str, int] = {}
    counter_pattern = re.compile(r"([a-zA-Z0-9_]+)=([0-9]+)")
    for line in stdout.splitlines():
        stripped = line.strip()
        if stripped.startswith("{"):
            try:
                value = json.loads(stripped)
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict) and "id" in value:
                records.append(value)
                continue
        for key, raw_value in counter_pattern.findall(stripped):
            counters[key] = int(raw_value)
    return records, counters


def kill_process_group(process: subprocess.Popen[str], sig: int) -> None:
    try:
        os.killpg(process.pid, sig)
    except (ProcessLookupError, PermissionError):
        try:
            process.send_signal(sig)
        except ProcessLookupError:
            pass



def run_agent(
    *,
    cli: Path,
    daemon: Path,
    upstream: Upstream,
    specs: dict[str, dict[str, Any]],
    phase: str,
    analysis_mode: str,
    workload: str,
    samples: int,
    warmup: int,
    root: Path,
    body_file: Path,
    sample_gap_ms: float,
) -> dict[str, Any]:
    burst_width = int(specs[workload].get("burst_width", 1))
    # Unix-domain socket paths have a small platform-defined limit. Keep the
    # diagnostic labels in the report, but never encode them in TRACEPRESS_HOME.
    run_namespace = f"r{time.time_ns()}"
    home: Path | None = None
    env: dict[str, str] | None = None
    daemon_process: subprocess.Popen[str] | None = None
    def attempt_environment(attempt_home: Path) -> dict[str, str]:
        attempt_env = os.environ.copy()
        attempt_env.update(
            {
                "TRACEPRESS_HOME": str(attempt_home),
                "TRACEPRESSD_BIN": str(daemon),
                "TRACEPRESS_UPSTREAM": upstream.url,
                "TRACEPRESS_CONTEXT_ANALYSIS": analysis_mode,
            }
        )
        return attempt_env

    def terminate_daemon_process(process: subprocess.Popen[str] | None) -> None:
        if process is None or process.poll() is not None:
            return
        kill_process_group(process, signal.SIGTERM)
        try:
            process.wait(timeout=10.0)
        except subprocess.TimeoutExpired:
            kill_process_group(process, signal.SIGKILL)
            process.wait(timeout=10.0)

    def start_daemon(attempt_home: Path, attempt_env: dict[str, str]) -> subprocess.Popen[str]:
        run_checked([str(cli), "init"], cwd=root, env=attempt_env, timeout_s=30.0)
        daemon_env = dict(attempt_env)
        daemon_env.update(
            {
                "TRACEPRESS_DATABASE": str(attempt_home / "tracepress.sqlite3"),
                "TRACEPRESS_CONTROL_SOCKET": str(attempt_home / "tracepress.sock"),
                "TRACEPRESS_CONTROL_CREDENTIAL": str(attempt_home / "control.cred"),
                "TRACEPRESS_DAEMON_READY": str(attempt_home / "daemon.ready"),
            }
        )
        process = subprocess.Popen(
            [str(daemon)],
            cwd=root,
            env=daemon_env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        deadline = time.monotonic() + 15.0
        try:
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise BenchmarkError(
                        f"tracepressd exited with status {process.returncode} before readiness"
                    )
                status = subprocess.run(
                    [str(cli), "daemon", "status"],
                    cwd=root,
                    env=attempt_env,
                    capture_output=True,
                    text=True,
                    timeout=30.0,
                    check=False,
                )
                if status.returncode == 0:
                    return process
                time.sleep(0.1)
        except BaseException:
            terminate_daemon_process(process)
            raise
        terminate_daemon_process(process)
        raise BenchmarkError("tracepressd did not become ready within 15 seconds")

    def stop_daemon(
        attempt_env: dict[str, str],
        process: subprocess.Popen[str] | None,
    ) -> None:
        try:
            run_checked([str(cli), "daemon", "stop"], cwd=root, env=attempt_env, timeout_s=30.0)
        except (BenchmarkError, subprocess.TimeoutExpired):
            pass
        terminate_daemon_process(process)

    start_error: BenchmarkError | None = None
    for attempt in range(5):
        attempt_home = root / f"{run_namespace}-h{attempt}"
        attempt_home.mkdir(parents=True, exist_ok=True)
        attempt_env = attempt_environment(attempt_home)
        try:
            attempt_process = start_daemon(attempt_home, attempt_env)
        except BenchmarkError as error:
            start_error = error
            stop_daemon(attempt_env, None)
            if attempt < 4:
                time.sleep(attempt + 1.0)
            continue
        home = attempt_home
        env = attempt_env
        daemon_process = attempt_process
        start_error = None
        break
    if start_error is not None or home is None or env is None or daemon_process is None:
        detail = start_error or BenchmarkError("no successful daemon start attempt")
        raise BenchmarkError(
            f"{phase}/{analysis_mode}/{workload} daemon start failed after 5 attempts: {detail}"
        ) from start_error

    stdout = ""
    stderr = ""
    rss_values: list[int] = []
    process: subprocess.Popen[str] | None = None
    monitor_stop = threading.Event()
    monitor_thread: threading.Thread | None = None
    tag = f"{phase}:{analysis_mode}:{workload}:{time.time_ns()}"
    try:
        process_env = dict(env)
        process_env.update(
            {
                "BENCH_REQUEST_FILE": str(body_file),
                "BENCH_SCENARIO": workload,
                "BENCH_RUN_TAG": tag,
                "BENCH_IDLE_SECONDS": "0.5",
                "BENCH_POST_RESPONSE_SETTLE_SECONDS": str(sample_gap_ms / 1000.0),
                "BENCH_SAMPLES": str(samples),
                "BENCH_WARMUP": str(warmup),
                "BENCH_BURST": str(burst_width),
                "BENCH_STREAMING": "1" if specs[workload]["content_type"] == "text/event-stream" else "0",
            }
        )
        process = subprocess.Popen(
            [str(cli), "run", sys.executable, "--", "-c", AGENT_SCRIPT],
            cwd=root,
            env=process_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        monitor_thread = threading.Thread(
            target=monitor_rss,
            args=(process, monitor_stop, rss_values),
            name="phase3-bench-rss",
            daemon=True,
        )
        monitor_thread.start()
        try:
            stdout, stderr = process.communicate(timeout=300.0)
        except subprocess.TimeoutExpired as error:
            kill_process_group(process, signal.SIGTERM)
            try:
                stdout, stderr = process.communicate(timeout=10.0)
            except subprocess.TimeoutExpired:
                kill_process_group(process, signal.SIGKILL)
                stdout, stderr = process.communicate(timeout=10.0)
            raise BenchmarkError(
                f"{phase}/{analysis_mode}/{workload} timed out after 300s"
            ) from error
        if process.returncode != 0:
            raise BenchmarkError(
                f"{phase}/{analysis_mode}/{workload} failed with status {process.returncode}: "
                f"{stderr[-512:]}"
            )
    finally:
        monitor_stop.set()
        if monitor_thread is not None:
            monitor_thread.join(timeout=2.0)
        stop_daemon(env, daemon_process)
        time.sleep(1.0)

    records, counters = parse_agent_output(stdout)
    measured = [record for record in records if record.get("measured") is True]
    expected = samples * (burst_width if burst_width > 1 else 1)
    if len(measured) != expected:
        raise BenchmarkError(
            f"{phase}/{workload} emitted {len(measured)} measured samples, expected {expected}"
        )
    successful_ids = [str(record.get("id")) for record in measured if not record.get("error")]
    upstream.wait_for(successful_ids, timeout_s=30.0)
    metric_rows: list[dict[str, float]] = []
    errors = 0
    missing = 0
    for record in measured:
        request_id = str(record.get("id"))
        upstream_metric = upstream.metric(request_id)
        if record.get("error") or upstream_metric is None:
            errors += 1
            if upstream_metric is None:
                missing += 1
            continue
        request_first = upstream_metric.get("request_first_ns")
        response_first = upstream_metric.get("response_first_ns")
        response_semantic = upstream_metric.get("response_semantic_ns")
        client_first = record.get("first_ns")
        client_semantic = record.get("semantic_ns")
        client_start = record.get("start_ns")
        if not all(
            isinstance(value, int)
            for value in (request_first, response_first, client_first, client_start)
        ):
            errors += 1
            continue
        row: dict[str, float] = {
            "dispatch_us": (request_first - client_start) / 1_000.0,
            "proxy_ttfb_us": (client_first - response_first) / 1_000.0,
            "duration_us": (record.get("end_ns", client_first) - client_start) / 1_000.0,
        }
        if isinstance(client_semantic, int) and isinstance(response_semantic, int):
            row["proxy_ttft_us"] = (client_semantic - response_semantic) / 1_000.0
        metric_rows.append(row)
    if not metric_rows:
        raise BenchmarkError(f"{phase}/{workload} produced no valid timing samples")

    def values(name: str) -> list[float]:
        return [row[name] for row in metric_rows if name in row]

    raw = {name: values(name) for name in ("dispatch_us", "proxy_ttfb_us", "proxy_ttft_us", "duration_us")}
    database = home / "tracepress.sqlite3"
    durable = durable_queue_metrics(database)
    rss = summarize([int(value) for value in rss_values])
    idle_rss = min(rss_values) if rss_values else None
    peak_rss = max(rss_values) if rss_values else None
    peak_rss_delta = None if idle_rss is None or peak_rss is None else peak_rss - idle_rss
    return {
        "phase": phase,
        "analysis_mode": analysis_mode,
        "burst_width": burst_width,
        "requested_samples": expected,
        "errors": errors,
        "missing_upstream_metrics": missing,
        "dispatch_us": summarize(raw["dispatch_us"]),
        "proxy_ttfb_us": summarize(raw["proxy_ttfb_us"]),
        "proxy_ttft_us": summarize(raw["proxy_ttft_us"]),
        "duration_us": summarize(raw["duration_us"]),
        "idle_rss_kib": idle_rss,
        "peak_rss_kib": peak_rss,
        "peak_rss_delta_kib": peak_rss_delta,
        "rss_kib": rss,
        "_raw": raw,
        "queue_observability": {
            "stdout_counters": counters,
            "durable": durable,
            "context_observer_backpressure_total": counters.get("context_observer_backpressure_total"),
        },
    }


def durable_queue_metrics(database: Path) -> dict[str, int | None]:
    """Read bounded lifecycle evidence from one isolated run database.

    ``None`` means the database/schema did not expose the requested evidence.  The
    acceptance gate deliberately rejects that state for current Phase 3 arms rather
    than treating an unobservable analysis path as a successful zero.
    """
    empty = {
        "context_snapshots": None,
        "terminal_context_snapshots": None,
        "observer_backpressure_snapshots": None,
        "context_analysis_dropped_events": None,
        "context_analysis_event_count": None,
        "phase2_provider_requests": None,
    }
    if not database.exists():
        return empty
    import sqlite3

    connection = sqlite3.connect(database)
    try:
        tables = {
            str(row[0])
            for row in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table'")
        }
        if "context_snapshots" not in tables:
            snapshots = None
            terminal = None
            backpressure = None
        else:
            snapshots = int(connection.execute("SELECT COUNT(*) FROM context_snapshots").fetchone()[0])
            terminal = int(
                connection.execute(
                    "SELECT COUNT(*) FROM context_snapshots "
                    "WHERE completed_at_us IS NOT NULL OR recovered_at_us IS NOT NULL"
                ).fetchone()[0]
            )
            backpressure = int(
                connection.execute(
                    "SELECT COUNT(*) FROM context_snapshots WHERE status = 'observer_backpressure'"
                ).fetchone()[0]
            )
        if "events" not in tables:
            dropped = None
            phase3_events = None
        else:
            dropped = int(
                connection.execute(
                    "SELECT COUNT(*) FROM events WHERE event_type = 'context.analysis.dropped'"
                ).fetchone()[0]
            )
            phase3_events = int(
                connection.execute(
                    "SELECT COUNT(*) FROM events WHERE event_type LIKE 'context.analysis.%'"
                ).fetchone()[0]
            )
        phase2_requests = (
            int(connection.execute("SELECT COUNT(*) FROM provider_requests").fetchone()[0])
            if "provider_requests" in tables
            else None
        )
        return {
            "context_snapshots": snapshots,
            "terminal_context_snapshots": terminal,
            "observer_backpressure_snapshots": backpressure,
            "context_analysis_dropped_events": dropped,
            "context_analysis_event_count": phase3_events,
            "phase2_provider_requests": phase2_requests,
        }
    except sqlite3.OperationalError:
        return empty
    finally:
        connection.close()


def _required_counter(counters: dict[str, int], name: str, label: str) -> int:
    value = counters.get(name)
    if not isinstance(value, int) or value < 0:
        raise BenchmarkError(f"{label} is missing non-negative counter {name}")
    return value


def validate_analysis_lifecycle(
    measured: dict[str, Any],
    *,
    phase: str,
    analysis_mode: str,
    workload: str,
    burst_width: int,
) -> None:
    """Reject an arm whose forwarding succeeds but whose analysis is unobservable."""
    label = f"{phase}/{analysis_mode}/{workload}"
    queue = measured.get("queue_observability")
    if not isinstance(queue, dict):
        raise BenchmarkError(f"{label} did not report queue/lifecycle observability")
    durable = queue.get("durable")
    if not isinstance(durable, dict):
        raise BenchmarkError(f"{label} did not report durable lifecycle evidence")
    phase2_requests = durable.get("phase2_provider_requests")
    if not isinstance(phase2_requests, int) or phase2_requests <= 0:
        raise BenchmarkError(f"{label} did not persist any Phase 2 provider request")

    snapshots = durable.get("context_snapshots")
    phase3_events = durable.get("context_analysis_event_count")
    if analysis_mode == "off":
        if snapshots != 0 or phase3_events != 0:
            raise BenchmarkError(
                f"{label} OFF arm persisted Phase 3 evidence: "
                f"snapshots={snapshots!r}, events={phase3_events!r}"
            )
        return
    if analysis_mode != "shadow":
        raise BenchmarkError(f"{label} has unknown analysis mode")

    counters = queue.get("stdout_counters")
    if not isinstance(counters, dict):
        raise BenchmarkError(f"{label} did not report Shadow counters")
    seen = _required_counter(counters, "analysis_requests_seen", label)
    complete = _required_counter(counters, "analysis_requests_complete", label)
    partial = _required_counter(counters, "analysis_requests_partial", label)
    dropped = _required_counter(counters, "analysis_requests_dropped", label)
    if seen == 0:
        raise BenchmarkError(f"{label} Shadow arm saw no eligible analysis request")
    if seen != complete + partial + dropped:
        raise BenchmarkError(
            f"{label} has non-terminal analysis partition: "
            f"seen={seen}, complete={complete}, partial={partial}, dropped={dropped}"
        )

    terminal_snapshots = durable.get("terminal_context_snapshots")
    dropped_events = durable.get("context_analysis_dropped_events")
    if not isinstance(terminal_snapshots, int) or not isinstance(dropped_events, int):
        raise BenchmarkError(f"{label} did not report durable Shadow terminal evidence")
    persisted = complete + partial
    if persisted > 0 and terminal_snapshots == 0:
        raise BenchmarkError(
            f"{label} reports persisted analysis outcomes without a durable terminal snapshot"
        )
    if dropped > 0 and dropped_events == 0:
        raise BenchmarkError(
            f"{label} reports dropped analyses without a durable context.analysis.dropped event"
        )
    if terminal_snapshots + dropped_events == 0:
        raise BenchmarkError(
            f"{label} Shadow arm has no durable terminal snapshot or matching drop event"
        )
    if workload == "context_1m" and burst_width == 1 and terminal_snapshots == 0:
        raise BenchmarkError(
            f"{label} sequential context_1m produced no durable snapshot; all analyses dropped"
        )


def run_self_test() -> None:
    """Exercise the lifecycle gate without building or starting the benchmark."""
    no_op = {
        "queue_observability": {
            "stdout_counters": {
                "analysis_requests_seen": 1,
                "analysis_requests_complete": 0,
                "analysis_requests_partial": 0,
                "analysis_requests_dropped": 0,
            },
            "durable": {
                "phase2_provider_requests": 1,
                "context_snapshots": 0,
                "terminal_context_snapshots": 0,
                "context_analysis_dropped_events": 0,
                "context_analysis_event_count": 0,
            },
        }
    }
    try:
        validate_analysis_lifecycle(
            no_op,
            phase="on",
            analysis_mode="shadow",
            workload="small_json",
            burst_width=1,
        )
    except BenchmarkError:
        pass
    else:
        raise AssertionError("a no-op Shadow analysis must be rejected")

    valid_on = {
        "queue_observability": {
            "stdout_counters": {
                "analysis_requests_seen": 1,
                "analysis_requests_complete": 1,
                "analysis_requests_partial": 0,
                "analysis_requests_dropped": 0,
            },
            "durable": {
                "phase2_provider_requests": 1,
                "context_snapshots": 1,
                "terminal_context_snapshots": 1,
                "context_analysis_dropped_events": 0,
                "context_analysis_event_count": 3,
            },
        }
    }
    validate_analysis_lifecycle(
        valid_on,
        phase="on",
        analysis_mode="shadow",
        workload="context_1m",
        burst_width=1,
    )
    valid_off = {
        "queue_observability": {
            "stdout_counters": {},
            "durable": {
                "phase2_provider_requests": 1,
                "context_snapshots": 0,
                "terminal_context_snapshots": 0,
                "context_analysis_dropped_events": 0,
                "context_analysis_event_count": 0,
            },
        }
    }
    validate_analysis_lifecycle(
        valid_off,
        phase="off_a",
        analysis_mode="off",
        workload="small_json",
        burst_width=1,
    )

    valid_drop = {
        "queue_observability": {
            "stdout_counters": {
                "analysis_requests_seen": 1,
                "analysis_requests_complete": 0,
                "analysis_requests_partial": 0,
                "analysis_requests_dropped": 1,
            },
            "durable": {
                "phase2_provider_requests": 1,
                "context_snapshots": 0,
                "terminal_context_snapshots": 0,
                "context_analysis_dropped_events": 1,
                "context_analysis_event_count": 1,
            },
        }
    }
    validate_analysis_lifecycle(
        valid_drop,
        phase="on",
        analysis_mode="shadow",
        workload="small_json",
        burst_width=1,
    )
    print("benchmark self-test: ok")


def bootstrap_median_delta(
    baseline: list[float], current: list[float], seed: int
) -> dict[str, float | bool | None]:
    if not baseline or not current:
        return {"lower_95_us": None, "upper_95_us": None, "significant_positive": False}
    import random

    randomizer = random.Random(seed)
    differences: list[float] = []
    for _ in range(4_000):
        baseline_sample = [baseline[randomizer.randrange(len(baseline))] for _ in baseline]
        current_sample = [current[randomizer.randrange(len(current))] for _ in current]
        differences.append(float(median(current_sample) - median(baseline_sample)))
    differences.sort()
    low_index = max(0, int(0.025 * len(differences)) - 1)
    high_index = min(len(differences) - 1, int(0.975 * len(differences)))
    lower = differences[low_index]
    upper = differences[high_index]
    return {
        "lower_95_us": round(lower, 3),
        "upper_95_us": round(upper, 3),
        "significant_positive": lower > 0.0,
    }


def compare_results(
    results: dict[str, dict[str, dict[str, Any]]],
    *,
    baseline_phase: str,
    current_phase: str,
) -> dict[str, Any]:
    comparison: dict[str, Any] = {
        "baseline_phase": baseline_phase,
        "current_phase": current_phase,
        "workloads": {},
        "significant_forwarding_regression": False,
    }
    for workload, phases in results.items():
        baseline = phases[baseline_phase]
        current = phases[current_phase]
        workload_comparison: dict[str, Any] = {}
        for metric_name in ("dispatch_us", "proxy_ttfb_us", "proxy_ttft_us", "duration_us"):
            baseline_values = baseline.get("_raw", {}).get(metric_name, [])
            current_values = current.get("_raw", {}).get(metric_name, [])
            if not baseline_values or not current_values:
                workload_comparison[metric_name] = {"available": False}
                continue
            baseline_median = float(baseline[metric_name]["median"])
            current_median = float(current[metric_name]["median"])
            delta = current_median - baseline_median
            relative = None if baseline_median == 0 else delta / baseline_median
            seed = sum(
                (index + 1) * ord(character)
                for index, character in enumerate(
                    f"{baseline_phase}:{current_phase}:{workload}:{metric_name}"
                )
            )
            ci = bootstrap_median_delta(
                [float(value) for value in baseline_values],
                [float(value) for value in current_values],
                seed=seed,
            )
            if ci["significant_positive"] and metric_name != "duration_us":
                comparison["significant_forwarding_regression"] = True
            workload_comparison[metric_name] = {
                "available": True,
                "baseline_median": round(baseline_median, 3),
                "current_median": round(current_median, 3),
                "delta_us": round(delta, 3),
                "relative_delta": None if relative is None else round(relative, 5),
                "bootstrap_median_delta_ci95": ci,
            }
        workload_comparison["idle_rss_kib_delta"] = (
            None
            if baseline.get("idle_rss_kib") is None or current.get("idle_rss_kib") is None
            else current["idle_rss_kib"] - baseline["idle_rss_kib"]
        )
        workload_comparison["peak_rss_kib_delta"] = (
            None
            if baseline.get("peak_rss_kib") is None or current.get("peak_rss_kib") is None
            else current["peak_rss_kib"] - baseline["peak_rss_kib"]
        )
        workload_comparison["peak_rss_idle_delta_kib"] = (
            None
            if baseline.get("peak_rss_delta_kib") is None or current.get("peak_rss_delta_kib") is None
            else current["peak_rss_delta_kib"] - baseline["peak_rss_delta_kib"]
        )
        comparison["workloads"][workload] = workload_comparison
    comparison["decision"] = (
        f"significant positive forwarding delta in {current_phase} versus {baseline_phase}; "
        "the phase gate is not acceptable"
        if comparison["significant_forwarding_regression"]
        else f"no significant positive forwarding delta in {current_phase} versus {baseline_phase}"
    )
    return comparison


def add_aa_envelope(
    comparison: dict[str, Any],
    envelopes: dict[tuple[str, str], list[dict[str, float]]],
) -> None:
    """Annotate a target comparison with the measured A/A noise envelope.

    The envelope is deliberately empirical: it is the largest absolute bootstrap CI bound
    measured by the requested A/A arms, with no externally chosen tolerance.
    """
    operational = True
    for workload, metrics in comparison["workloads"].items():
        workload_operational = True
        for metric_name in ("dispatch_us", "proxy_ttfb_us", "proxy_ttft_us", "duration_us"):
            metric = metrics[metric_name]
            if not metric.get("available"):
                continue
            measured = envelopes.get((workload, metric_name), [])
            if not measured:
                metric["operationally_acceptable"] = False
                workload_operational = False
                continue
            absolute = max(item["absolute_us"] for item in measured)
            relative = max(item["relative"] for item in measured)
            ci = metric["bootstrap_median_delta_ci95"]
            lower = float(ci["lower_95_us"])
            upper = float(ci["upper_95_us"])
            crosses_zero = lower <= 0.0 <= upper
            within_envelope = abs(float(metric["delta_us"])) <= absolute
            metric["aa_envelope_absolute_us"] = round(absolute, 3)
            metric["aa_envelope_relative"] = round(relative, 5)
            metric["crosses_zero"] = crosses_zero
            metric["within_aa_envelope"] = within_envelope
            # A CI wholly below zero is an improvement, not a forwarding regression. Only a
            # positive change needs to be explained by the measured A/A noise envelope.
            metric["operationally_acceptable"] = (
                upper <= 0.0 or crosses_zero or within_envelope
            )
            if not metric["operationally_acceptable"]:
                workload_operational = False
        metrics["operationally_acceptable"] = workload_operational
        operational = operational and workload_operational
    comparison["operationally_acceptable"] = operational
    comparison["operational_decision"] = (
        "acceptable: every target CI crosses zero or stays within measured A/A envelope"
        if operational
        else "reject: a target delta exceeds its measured A/A envelope without crossing zero"
    )


def aa_envelopes(
    results: dict[str, dict[str, dict[str, Any]]],
    *,
    baseline_phase: str,
    current_phase: str,
) -> tuple[dict[str, Any], dict[tuple[str, str], list[dict[str, float]]]]:
    """Return the A/A comparison and per-metric absolute/relative noise envelope."""
    comparison = compare_results(
        results,
        baseline_phase=baseline_phase,
        current_phase=current_phase,
    )
    envelopes: dict[tuple[str, str], list[dict[str, float]]] = {}
    for workload, metrics in comparison["workloads"].items():
        for metric_name in ("dispatch_us", "proxy_ttfb_us", "proxy_ttft_us", "duration_us"):
            metric = metrics[metric_name]
            if not metric.get("available"):
                continue
            baseline_median = abs(float(metric["baseline_median"]))
            ci = metric["bootstrap_median_delta_ci95"]
            absolute = max(abs(float(ci["lower_95_us"])), abs(float(ci["upper_95_us"])))
            relative = 0.0 if baseline_median == 0.0 else absolute / baseline_median
            metric["aa_envelope_absolute_us"] = round(absolute, 3)
            metric["aa_envelope_relative"] = round(relative, 5)
            envelopes.setdefault((workload, metric_name), []).append(
                {"absolute_us": absolute, "relative": relative}
            )
    return comparison, envelopes


def render_table(report: dict[str, Any]) -> str:
    lines = [
        "Frozen baseline vs current context analysis OFF/ON",
        "workload                 metric       baseline med/p95    OFF med/p95        ON med/p95         ON-OFF",
        "-----------------------  -----------  ------------------  ----------------  ----------------  --------",
    ]
    for workload, phases in report["results"].items():
        for metric_name, label in (
            ("dispatch_us", "dispatch us"),
            ("proxy_ttfb_us", "TTFB us"),
            ("proxy_ttft_us", "TTFT us"),
        ):
            values = [phases[name].get(metric_name, {}) for name in ("baseline", "off", "on")]
            if any(not value.get("sample_count") for value in values):
                continue
            formatted = [f"{value['median']:.3f}/{value['p95']:.3f}" for value in values]
            delta = values[2]["median"] - values[1]["median"]
            lines.append(
                f"{workload:23}  {label:11}  {formatted[0]:18}  {formatted[1]:16}  "
                f"{formatted[2]:16}  {delta:+.3f}"
            )
        idle = [phases[name].get("idle_rss_kib") for name in ("baseline", "off", "on")]
        delta = None if idle[1] is None or idle[2] is None else idle[2] - idle[1]
        lines.append(
            f"{workload:23}  {'idle RSS KiB':11}  {str(idle[0]):18}  {str(idle[1]):16}  "
            f"{str(idle[2]):16}  {'' if delta is None else f'{delta:+d}'}"
        )
    lines.append("")
    comparison = report["comparison"]
    lines.append(f"OFF versus baseline: {comparison['off_vs_baseline']['decision']}")
    lines.append(f"ON versus OFF: {comparison['on_vs_off']['decision']}")
    lines.append(
        f"OFF versus baseline operational gate: {comparison['off_vs_baseline']['operational_decision']}"
    )
    lines.append(
        f"ON versus OFF operational gate: {comparison['on_vs_off']['operational_decision']}"
    )
    lines.append(
        "A/A baseline and OFF envelopes are measured per workload/metric; target deltas report "
        "absolute and relative values without an invented tolerance."
    )
    lines.append(
        "Queue counters/errors are reported for every arm; analysis concurrency cap=2, "
        "deferred analysis cap=32 items/64 MiB, and durable ingestion cap=4. Idle RSS ON-OFF "
        "is the reported process-resident delta."
    )
    return "\n".join(lines)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--baseline-tag", default=BASELINE_TAG)
    parser.add_argument(
        "--expected-baseline-commit",
        default=EXPECTED_BASELINE_COMMIT,
        help=(
            "expected commit prefix for the baseline tag "
            f"(default: {EXPECTED_BASELINE_COMMIT})"
        ),
    )
    parser.add_argument("--samples", type=int, default=DEFAULT_SAMPLES)
    parser.add_argument("--warmup", type=int, default=DEFAULT_WARMUP)
    parser.add_argument("--burst", type=int, help=argparse.SUPPRESS)
    parser.add_argument(
        "--burst-widths",
        default=",".join(str(width) for width in DEFAULT_BURST_WIDTHS),
        help="comma-separated concurrent burst widths (default: 32,64,72)",
    )
    parser.add_argument(
        "--sample-gap-ms",
        type=float,
        default=100.0,
        help="post-response settle gap applied equally to every arm (default: 100)",
    )
    parser.add_argument(
        "--workloads",
        default=None,
        help="comma-separated workload names to run (default: all workloads)",
    )
    parser.add_argument("--json-output", type=Path)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="exercise lifecycle acceptance checks without building or running the benchmark",
    )
    args = parser.parse_args()
    if args.samples < 1 or args.warmup < 0 or args.sample_gap_ms < 0:
        parser.error("samples must be positive, warmup non-negative, and sample gap non-negative")
    try:
        widths = (args.burst,) if args.burst is not None else tuple(
            int(part.strip()) for part in args.burst_widths.split(",") if part.strip()
        )
    except ValueError:
        parser.error("burst widths must be comma-separated positive integers")
    if not widths or any(width < 1 for width in widths):
        parser.error("burst widths must contain positive integers")
    args.burst_widths = tuple(sorted(set(widths)))
    if args.workloads is not None:
        selected = tuple(name.strip() for name in args.workloads.split(",") if name.strip())
        if not selected:
            parser.error("workloads must contain at least one name")
        args.workloads = tuple(dict.fromkeys(selected))
    return args


def main() -> int:
    args = parse_args()
    if args.self_test:
        run_self_test()
        return 0
    repo = args.repo_root.resolve()
    baseline_commit = git_output(repo, "rev-parse", f"{args.baseline_tag}^{{commit}}")
    if not baseline_commit.startswith(args.expected_baseline_commit):
        raise BenchmarkError(
            f"baseline tag {args.baseline_tag} resolved to {baseline_commit}, "
            f"expected {args.expected_baseline_commit}"
        )
    current_commit = git_output(repo, "rev-parse", "HEAD")
    current_worktree_dirty = bool(git_output(repo, "status", "--porcelain"))
    specs, bodies = workloads(args.burst_widths)
    available_workloads = tuple(specs)
    selected_workloads = available_workloads if args.workloads is None else args.workloads
    unknown_workloads = sorted(set(selected_workloads) - set(available_workloads))
    if unknown_workloads:
        raise BenchmarkError(
            f"unknown workload(s): {', '.join(unknown_workloads)}; "
            f"available: {', '.join(available_workloads)}"
        )
    specs = {name: specs[name] for name in selected_workloads}
    bodies = {name: bodies[name] for name in selected_workloads}
    args.workloads = selected_workloads
    report: dict[str, Any] = {
        "metadata": {
            "baseline_tag": args.baseline_tag,
            "baseline_commit": baseline_commit,
            "expected_baseline_commit": args.expected_baseline_commit,
            "current_worktree_dirty": current_worktree_dirty,
            "current_commit": current_commit,
            "compiler_profile": "release (Cargo workspace profile; --locked)",
            "samples_per_workload": args.samples,
            "warmup_per_workload": args.warmup,
            "burst_widths": args.burst_widths,
            "workloads": args.workloads,
            "upstream": "deterministic local HTTP/1.1 socket server",
            "phase_arms": {
                "baseline_a": "selected baseline tag; context-analysis env ignored; first A/A arm",
                "off_a": "current checkout with TRACEPRESS_CONTEXT_ANALYSIS=off; first A/A arm",
                "on": "current checkout with TRACEPRESS_CONTEXT_ANALYSIS=shadow",
                "off_b": "current checkout with TRACEPRESS_CONTEXT_ANALYSIS=off; second A/A arm",
                "baseline_b": "selected baseline tag; context-analysis env ignored; second A/A arm",
            },
            "arm_order": ["baseline_a", "off_a", "on", "off_b", "baseline_b"],
            "aa_comparisons": {
                "baseline": "baseline_a versus baseline_b",
                "off": "off_a versus off_b",
            },
            "post_response_settle_seconds": args.sample_gap_ms / 1000.0,
            "metric_definitions": {
                "dispatch_us": "client start to deterministic upstream first request byte",
                "proxy_ttfb_us": "deterministic upstream first response body byte to client first body byte",
                "proxy_ttft_us": "deterministic upstream semantic SSE chunk to client semantic SSE chunk",
                "duration_us": "client start to client response end",
            },
            "request_bytes": {name: len(body) for name, body in bodies.items()},
            "forwarding_bound": 64,
            "context_analysis_concurrency_cap": 2,
            "context_ingestion_queue_cap": 4,
            "deferred_analysis_queue_item_cap": 32,
            "deferred_analysis_queue_byte_cap": 64 * 1024 * 1024,
            "request_errors_must_be_zero": True,
        },
        "results": {},
    }
    # Keep the temporary root short because each isolated run contains a Unix
    # control socket below it. The labels remain in report metadata and errors.
    with tempfile.TemporaryDirectory(prefix="tp3-") as temporary:
        temporary_root = Path(temporary)
        baseline_worktree = temporary_root / "baseline-worktree"
        baseline_target = temporary_root / "baseline-target"
        current_target = temporary_root / "current-target"
        try:
            run_checked(
                ["git", "worktree", "add", "--detach", str(baseline_worktree), baseline_commit],
                cwd=repo,
                env=os.environ.copy(),
                timeout_s=120.0,
            )
            baseline_cli, baseline_daemon = build_checkout(baseline_worktree, baseline_target)
            current_cli, current_daemon = build_checkout(repo, current_target)
            upstream = Upstream(specs)
            try:
                arms = (
                    ("baseline_a", baseline_cli, baseline_daemon, "off"),
                    ("off_a", current_cli, current_daemon, "off"),
                    ("on", current_cli, current_daemon, "shadow"),
                    ("off_b", current_cli, current_daemon, "off"),
                    ("baseline_b", baseline_cli, baseline_daemon, "off"),
                )
                for phase, cli, daemon, analysis_mode in arms:
                    for workload, body in bodies.items():
                        body_file = temporary_root / f"{phase}-{workload}.json"
                        body_file.write_bytes(body)
                        measured = run_agent(
                            cli=cli,
                            daemon=daemon,
                            upstream=upstream,
                            specs=specs,
                            phase=phase,
                            analysis_mode=analysis_mode,
                            workload=workload,
                            samples=args.samples,
                            warmup=args.warmup,
                            root=temporary_root,
                            body_file=body_file,
                            sample_gap_ms=args.sample_gap_ms,
                        )
                        if measured["errors"] != 0:
                            raise BenchmarkError(
                                f"{phase}/{workload} reported {measured['errors']} request errors"
                            )
                        if phase in {"off_a", "off_b", "on"}:
                            validate_analysis_lifecycle(
                                measured,
                                phase=phase,
                                analysis_mode=analysis_mode,
                                workload=workload,
                                burst_width=int(specs[workload].get("burst_width", 1)),
                            )
                        # Keep raw timing samples transiently for bootstrap comparison, never in
                        # output. The timing summaries above remain the public report values.
                        report["results"].setdefault(workload, {})[phase] = measured
            finally:
                upstream.close()
            for phases in report["results"].values():
                phases["baseline"] = phases["baseline_a"]
                phases["off"] = phases["off_a"]
            baseline_aa, baseline_envelope = aa_envelopes(
                report["results"],
                baseline_phase="baseline_a",
                current_phase="baseline_b",
            )
            off_aa, off_envelope = aa_envelopes(
                report["results"],
                baseline_phase="off_a",
                current_phase="off_b",
            )
            combined_envelope = {
                key: baseline_envelope.get(key, []) + off_envelope.get(key, [])
                for key in set(baseline_envelope) | set(off_envelope)
            }
            off_vs_baseline = compare_results(
                report["results"], baseline_phase="baseline_a", current_phase="off_a"
            )
            on_vs_off = compare_results(
                report["results"], baseline_phase="off_a", current_phase="on"
            )
            add_aa_envelope(off_vs_baseline, combined_envelope)
            add_aa_envelope(on_vs_off, off_envelope)
            operationally_acceptable = (
                off_vs_baseline["operationally_acceptable"]
                and on_vs_off["operationally_acceptable"]
            )
            report["comparison"] = {
                "off_vs_baseline": off_vs_baseline,
                "on_vs_off": on_vs_off,
                "aa": {
                    "baseline_vs_baseline": baseline_aa,
                    "off_vs_off": off_aa,
                },
                "operationally_acceptable": operationally_acceptable,
                "significant_forwarding_regression": (
                    off_vs_baseline["significant_forwarding_regression"]
                    or on_vs_off["significant_forwarding_regression"]
                ),
                "decision": (
                    "accept: ON-OFF and OFF-baseline cross zero or remain within measured A/A noise"
                    if operationally_acceptable
                    else "reject: ON-OFF or OFF-baseline exceeds measured A/A noise without crossing zero"
                ),
            }
        finally:
            try:
                run_checked(
                    ["git", "worktree", "remove", "--force", str(baseline_worktree)],
                    cwd=repo,
                    env=os.environ.copy(),
                    timeout_s=120.0,
                )
            except BenchmarkError:
                # The temporary directory cleanup still removes files; a later invocation can
                # safely prune a stale registration with `git worktree prune`.
                pass
    for phases in report["results"].values():
        for phase in phases.values():
            phase.pop("_raw", None)
    document = json.dumps(report, indent=2, sort_keys=True)
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(document + "\n", encoding="utf-8")
    print(render_table(report))
    print("\nJSON")
    print(document)
    return 0




if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (BenchmarkError, subprocess.TimeoutExpired) as error:
        print(f"benchmark failed: {error}", file=sys.stderr)
        raise SystemExit(2)
''
