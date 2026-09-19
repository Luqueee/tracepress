#!/usr/bin/env python3
"""Contract tests for the Phase 6.7 ephemeral cache-key equality cohort."""

from __future__ import annotations

from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading
import unittest


def load(name: str, filename: str):
    path = Path(__file__).with_name(filename)
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CHILD = load("cache_key_equality_child_020", "run_cache_key_equality_child_020.py")
MODULE = load("cache_key_equality_020", "run_cache_key_equality_020.py")


class EchoHandler(BaseHTTPRequestHandler):
    received = b""

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers["content-length"])
        type(self).received = self.rfile.read(length)
        response = b'{"status":"ok"}'
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def synthetic_cohort() -> dict[str, object]:
    records = []
    runs = []
    class_index = 0
    for round_index, order in enumerate(CHILD.balanced_schedule()):
        for position, arm in enumerate(order):
            runs.append(
                {
                    "arm": arm,
                    "round": round_index,
                    "position": position,
                    "return_code": 0,
                    "timed_out": False,
                }
            )
            for request_index in range(2):
                records.append(
                    {
                        "arm": arm,
                        "round": round_index,
                        "request_index": request_index,
                        "cache_key_status": "present",
                        "equality_class": class_index,
                    }
                )
            class_index += 1
    return {
        "runs": runs,
        "cache_key_equality": {
            "records": records,
            "unique_present_key_classes": 18,
            "raw_key_values_persisted": False,
            "reusable_key_hashes_persisted": False,
        },
    }


def create_database(path: Path) -> None:
    with closing(sqlite3.connect(path)) as connection:
        connection.executescript(
            """
            CREATE TABLE provider_requests(request_id TEXT PRIMARY KEY);
            CREATE TABLE provider_attempts(
                attempt_id TEXT PRIMARY KEY, request_id TEXT NOT NULL, ordinal INTEGER NOT NULL
            );
            CREATE TABLE provider_usage(
                attempt_id TEXT PRIMARY KEY, input_total INTEGER, input_cached INTEGER,
                input_uncached INTEGER, output_total INTEGER, output_reasoning INTEGER
            );
            """
        )
        for index in range(36):
            request = f"request-{index}"
            attempt = f"attempt-{index}"
            connection.execute("INSERT INTO provider_requests VALUES (?)", (request,))
            connection.execute(
                "INSERT INTO provider_attempts VALUES (?, ?, 0)", (attempt, request)
            )
            connection.execute(
                "INSERT INTO provider_usage VALUES (?, 1000, 800, 200, 50, 20)",
                (attempt,),
            )
        connection.commit()


class CacheKeyEqualityTests(unittest.TestCase):
    def test_state_emits_only_ordinal_equality_classes(self) -> None:
        state = CHILD.EqualityState()
        state.begin_run("hooks_implicit_enabled", 0)
        state.observe(b'{"prompt_cache_key":"CACHE_CANARY"}')
        state.observe(b'{"prompt_cache_key":"CACHE_CANARY"}')
        state.begin_run("hooks_explicit_enabled", 0)
        state.observe(b'{"prompt_cache_key":"SECOND_CANARY"}')
        report = state.safe_report()
        serialized = json.dumps(report)

        self.assertEqual(
            [record["equality_class"] for record in report["records"]], [0, 0, 1]
        )
        self.assertNotIn("CACHE_CANARY", serialized)
        self.assertNotIn("SECOND_CANARY", serialized)
        self.assertFalse(report["raw_key_values_persisted"])

    def test_observer_forwards_request_bytes_exactly(self) -> None:
        echo = ThreadingHTTPServer(("127.0.0.1", 0), EchoHandler)
        echo_thread = threading.Thread(target=echo.serve_forever, daemon=True)
        echo_thread.start()
        state = CHILD.EqualityState()
        state.begin_run("hooks_implicit_enabled", 0)
        observer = CHILD.EqualityObserverServer(
            f"http://127.0.0.1:{echo.server_address[1]}/v1", state
        )
        observer_thread = threading.Thread(target=observer.serve_forever, daemon=True)
        observer_thread.start()
        decoded_body = b'{"prompt_cache_key":"CACHE_CANARY","input":"PROMPT_CANARY"}'
        compressed = subprocess.run(
            ["zstd", "-cq"], input=decoded_body, capture_output=True, check=True
        ).stdout
        try:
            connection = http.client.HTTPConnection("127.0.0.1", observer.server_address[1])
            connection.request(
                "POST", "/v1/responses", body=compressed,
                headers={"content-encoding": "zstd"},
            )
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            self.assertEqual(response.read(), b'{"status":"ok"}')
            connection.close()
        finally:
            observer.shutdown()
            observer.server_close()
            echo.shutdown()
            echo.server_close()
            observer_thread.join(timeout=5)
            echo_thread.join(timeout=5)

        self.assertEqual(EchoHandler.received, compressed)
        self.assertNotIn("CACHE_CANARY", json.dumps(state.safe_report()))

    def test_child_command_changes_only_controlled_feature_state(self) -> None:
        provider = ["-c", 'model_provider="tracepress_subscription"']
        command = CHILD.child_command(
            Path("/real/codex"), provider, "http://127.0.0.1:1234/v1",
            "hooks_explicit_disabled", 0,
        )
        self.assertEqual(command[0], "/real/codex")
        self.assertIn("--disable", command)
        self.assertNotIn("--enable", command)
        self.assertIn("hooks", command)
        self.assertIn("--ephemeral", command)

    def test_zstd_analysis_decode_has_a_hard_output_bound(self) -> None:
        oversized = b"x" * (CHILD.MAX_ANALYSIS_BYTES + 1)
        compressed = subprocess.run(
            ["zstd", "-cq"], input=oversized, capture_output=True, check=True
        ).stdout
        self.assertEqual(CHILD.decode_zstd_bounded(compressed), b"")

    def test_unique_per_run_classes_are_classified(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            database = Path(raw) / "evidence.sqlite3"
            create_database(database)
            analysis = MODULE.analyze(synthetic_cohort(), database)

        self.assertTrue(analysis["execution_contract_complete"])
        self.assertTrue(analysis["record_contract_complete"])
        self.assertEqual(analysis["unique_equality_classes"], 18)
        self.assertTrue(analysis["within_run_class_stable"])
        self.assertEqual(analysis["classification"], "session_scoped_cache_key_classes")
        self.assertTrue(
            all(value == 0 for value in analysis["matched_round_cross_arm_equalities"].values())
        )

    def test_report_never_claims_provider_causality(self) -> None:
        analysis = {
            "execution_contract_complete": True,
            "record_contract_complete": True,
            "classification": "session_scoped_cache_key_classes",
        }
        report = MODULE.build_report(analysis, "codex-cli test")
        self.assertFalse(report["gate"]["provider_effect_active"])
        self.assertFalse(report["gate"]["cache_key_values_persisted"])
        self.assertFalse(report["gate"]["instruction_policy_active"])

    def test_persisted_report_contains_only_aggregate_equality(self) -> None:
        path = (
            Path(__file__).parents[1]
            / "reports/cache-key-equality-020/TRACEPRESS_CACHE_KEY_EQUALITY_020.json"
        )
        report = json.loads(path.read_text(encoding="utf-8"))
        equality = report["cache_key_equality"]
        serialized = json.dumps(report)

        self.assertEqual(report["gate"]["decision"], "session_scoped_cache_key_classes")
        self.assertEqual(equality["cache_key_status_counts"], {"present": 36})
        self.assertEqual(equality["unique_equality_classes"], 18)
        self.assertTrue(equality["within_run_class_stable"])
        self.assertNotIn("records", equality)
        self.assertNotIn('"equality_class":', serialized)
        for canary in ("CACHE_CANARY", "PROMPT_CANARY", "AUTH_CANARY"):
            self.assertNotIn(canary, serialized)


if __name__ == "__main__":
    unittest.main()
