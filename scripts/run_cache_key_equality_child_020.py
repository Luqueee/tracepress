#!/usr/bin/env python3
"""Run controlled Codex arms through an ephemeral cache-key equality observer."""

from __future__ import annotations

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import json
import os
from pathlib import Path
import subprocess
import threading
from typing import Any
from urllib.parse import urlsplit


MODEL = "gpt-5.6-luna"
ARM_NAMES = (
    "hooks_implicit_enabled",
    "hooks_explicit_enabled",
    "hooks_explicit_disabled",
)
SEARCH_PATTERNS = ("Result<", "fn ")
MAX_ANALYSIS_BYTES = 8 * 1024 * 1024


def balanced_schedule() -> tuple[tuple[str, ...], ...]:
    implicit, enabled, disabled = ARM_NAMES
    return (
        (implicit, enabled, disabled),
        (enabled, disabled, implicit),
        (disabled, implicit, enabled),
        (implicit, disabled, enabled),
        (disabled, enabled, implicit),
        (enabled, implicit, disabled),
    )


def controlled_prompt(pattern: str) -> str:
    return (
        "Inspect this public repository. Use the shell exactly once to run a bounded ripgrep "
        f"search for {pattern!r} in Rust files, then answer with only the aggregate number of "
        "matching lines and unique files. Do not modify files."
    )


class EqualityState:
    """Own raw keys only for the lifetime of this child process."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._keys: list[str] = []
        self._current_arm: str | None = None
        self._current_round: int | None = None
        self._request_index = 0
        self._records: list[dict[str, Any]] = []

    def begin_run(self, arm: str, round_index: int) -> None:
        with self._lock:
            self._current_arm = arm
            self._current_round = round_index
            self._request_index = 0

    def observe(self, body: bytes, content_encoding: str | None = None) -> None:
        status = "malformed"
        key: str | None = None
        analysis_body = body
        if content_encoding is not None and content_encoding.lower() == "zstd":
            analysis_body = decode_zstd_bounded(body)
        try:
            value = json.loads(analysis_body)
            if not isinstance(value, dict) or "prompt_cache_key" not in value:
                status = "absent"
            elif value["prompt_cache_key"] is None:
                status = "null"
            elif isinstance(value["prompt_cache_key"], str):
                status = "present"
                key = value["prompt_cache_key"]
            else:
                status = "invalid_type"
        except (UnicodeDecodeError, json.JSONDecodeError):
            pass
        with self._lock:
            equality_class: int | None = None
            if key is not None:
                try:
                    equality_class = self._keys.index(key)
                except ValueError:
                    equality_class = len(self._keys)
                    self._keys.append(key)
            self._records.append(
                {
                    "arm": self._current_arm,
                    "round": self._current_round,
                    "request_index": self._request_index,
                    "cache_key_status": status,
                    "equality_class": equality_class,
                }
            )
            self._request_index += 1

    def safe_report(self) -> dict[str, Any]:
        with self._lock:
            return {
                "records": list(self._records),
                "unique_present_key_classes": len(self._keys),
                "raw_key_values_persisted": False,
                "reusable_key_hashes_persisted": False,
            }


def decode_zstd_bounded(body: bytes) -> bytes:
    """Decode into memory with a hard output bound and no temporary plaintext file."""
    try:
        process = subprocess.Popen(
            ["zstd", "-dc"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
    except OSError:
        return b""
    assert process.stdin is not None and process.stdout is not None

    def feed() -> None:
        try:
            process.stdin.write(body)
        except (BrokenPipeError, OSError):
            pass
        finally:
            process.stdin.close()

    writer = threading.Thread(target=feed, daemon=True)
    writer.start()
    decoded = process.stdout.read(MAX_ANALYSIS_BYTES + 1)
    if len(decoded) > MAX_ANALYSIS_BYTES:
        process.kill()
    try:
        return_code = process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
        return_code = -1
    writer.join(timeout=5)
    process.stdout.close()
    return decoded if return_code == 0 and len(decoded) <= MAX_ANALYSIS_BYTES else b""


class EqualityObserverServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, target_base: str, state: EqualityState) -> None:
        parsed = urlsplit(target_base)
        if parsed.scheme != "http" or not parsed.hostname or not parsed.port:
            raise ValueError("Tracepress observer target must be an explicit local HTTP endpoint")
        self.target_host = parsed.hostname
        self.target_port = parsed.port
        self.state = state
        super().__init__(("127.0.0.1", 0), EqualityHandler)


class EqualityHandler(BaseHTTPRequestHandler):
    server: EqualityObserverServer
    protocol_version = "HTTP/1.0"

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler API
        length = self.headers.get("content-length")
        if length is None:
            self.send_error(411)
            return
        try:
            body_length = int(length)
        except ValueError:
            self.send_error(400)
            return
        body = self.rfile.read(body_length)
        self.server.state.observe(body, self.headers.get("content-encoding"))
        headers = {
            name: value
            for name, value in self.headers.items()
            if name.lower() not in {"host", "connection", "transfer-encoding", "content-length"}
        }
        headers["content-length"] = str(len(body))
        connection = http.client.HTTPConnection(
            self.server.target_host, self.server.target_port, timeout=180
        )
        try:
            connection.request("POST", self.path, body=body, headers=headers)
            upstream = connection.getresponse()
            self.send_response(upstream.status, upstream.reason)
            for name, value in upstream.getheaders():
                if name.lower() not in {
                    "connection",
                    "content-length",
                    "transfer-encoding",
                    "keep-alive",
                }:
                    self.send_header(name, value)
            self.send_header("connection", "close")
            self.end_headers()
            while chunk := upstream.read(16 * 1024):
                self.wfile.write(chunk)
                self.wfile.flush()
        finally:
            connection.close()

    def log_message(self, _format: str, *_args: object) -> None:
        return


def parse_args() -> tuple[argparse.Namespace, list[str]]:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cohort-output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--real-codex", type=Path, required=True)
    options, provider_args = parser.parse_known_args()
    return options, provider_args


def provider_base_url(provider_args: list[str]) -> str:
    prefix = "model_providers.tracepress_subscription.base_url="
    values: list[str] = []
    for index, argument in enumerate(provider_args[:-1]):
        if argument == "-c" and provider_args[index + 1].startswith(prefix):
            raw = provider_args[index + 1][len(prefix) :]
            values.append(json.loads(raw))
    if len(values) != 1:
        raise ValueError("expected exactly one Tracepress subscription base URL")
    return values[0]


def child_command(
    real_codex: Path,
    provider_args: list[str],
    observer_base: str,
    arm: str,
    round_index: int,
) -> list[str]:
    command = [str(real_codex), "-a", "never", "exec", *provider_args]
    command.extend(
        [
            "-c",
            f'model_providers.tracepress_subscription.base_url="{observer_base}"',
        ]
    )
    if arm == "hooks_explicit_enabled":
        command.extend(["--enable", "hooks"])
    elif arm == "hooks_explicit_disabled":
        command.extend(["--disable", "hooks"])
    command.extend(
        [
            "--ephemeral",
            "-m",
            MODEL,
            "-s",
            "read-only",
            "--skip-git-repo-check",
            controlled_prompt(SEARCH_PATTERNS[round_index // 3]),
        ]
    )
    return command


def main() -> int:
    options, provider_args = parse_args()
    target = provider_base_url(provider_args)
    state = EqualityState()
    server = EqualityObserverServer(target, state)
    observer_base = f"http://127.0.0.1:{server.server_address[1]}/v1"
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    runs: list[dict[str, Any]] = []
    try:
        for round_index, order in enumerate(balanced_schedule()):
            for position, arm in enumerate(order):
                state.begin_run(arm, round_index)
                try:
                    result = subprocess.run(
                        child_command(
                            options.real_codex,
                            provider_args,
                            observer_base,
                            arm,
                            round_index,
                        ),
                        check=False,
                        capture_output=True,
                        text=True,
                        timeout=options.timeout,
                    )
                    return_code = result.returncode
                    timed_out = False
                except subprocess.TimeoutExpired:
                    return_code = 124
                    timed_out = True
                runs.append(
                    {
                        "arm": arm,
                        "round": round_index,
                        "position": position,
                        "return_code": return_code,
                        "timed_out": timed_out,
                    }
                )
    finally:
        server.shutdown()
        server.server_close()
        server_thread.join(timeout=5)
    report = {
        "runs": runs,
        "cache_key_equality": state.safe_report(),
        "privacy": {
            "raw_request_bodies_persisted": False,
            "raw_key_values_persisted": False,
            "reusable_key_hashes_persisted": False,
            "child_stdout_persisted": False,
            "child_stderr_persisted": False,
        },
    }
    options.cohort_output.parent.mkdir(parents=True, exist_ok=True)
    options.cohort_output.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0 if all(run["return_code"] == 0 for run in runs) else 1


if __name__ == "__main__":
    raise SystemExit(main())
