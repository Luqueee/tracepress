#!/usr/bin/env python3
"""Contract tests for the metadata-only scheduler sidecar collector."""

from __future__ import annotations

import importlib.util
import json
import stat
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "scripts" / "collect_scheduler_sidecar.py"
SPEC = importlib.util.spec_from_file_location("collect_scheduler_sidecar", PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {PATH}")
COLLECTOR = importlib.util.module_from_spec(SPEC)
sys.modules[PATH.stem] = COLLECTOR
SPEC.loader.exec_module(COLLECTOR)


SIMULATOR = r'''
import json, os, sys, time
session_id, request_count, delay = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
run_id = os.environ["TRACEPRESS_MEASUREMENT_RUN_ID"]
print("TRACEPRESS_MEASUREMENT_METADATA=" + json.dumps({
    "measurement_run_id": run_id,
    "session_id": session_id,
    "tracepress_pid": os.getpid(),
    "started_at": "simulated-start",
}, separators=(",", ":")), flush=True)
print("Bearer TRACEPRESS_SUBSCRIPTION_CANARY prompt tool-result", flush=True)
print("stderr TRACEPRESS_SUBSCRIPTION_CANARY", file=sys.stderr, flush=True)
time.sleep(delay)
print("TRACEPRESS_SCHEDULER_METRICS=" + json.dumps({
    "deferred_queue_items": 0,
    "deferred_queue_bytes": 0,
    "deferred_high_water_items": 2,
    "deferred_high_water_bytes": 128,
    "analysis_admitted_total": request_count,
    "analysis_deferred_total": 0,
    "processed_deferred_total": request_count,
    "backlog_capacity_drops": 0,
    "analysis_wait_us": 17,
}, separators=(",", ":")), flush=True)
print(f"analysis_requests_seen={request_count}", flush=True)
print(f"analysis_requests_complete={request_count}", flush=True)
print("analysis_requests_partial=0", flush=True)
print("analysis_requests_dropped=0", flush=True)
print(f"correlation_eligible={request_count}", flush=True)
print(f"correlation_correlated={request_count}", flush=True)
'''


class SidecarCollectorTests(unittest.TestCase):
    def test_atomic_capture_keeps_explicit_run_and_session_identity(self) -> None:
        with TemporaryDirectory() as directory:
            result = COLLECTOR.collect_scheduler_metrics(
                [sys.executable, "-c", SIMULATOR, "session-a", "3", "0"],
                Path(directory),
            )

            self.assertTrue(result["capture_complete"])
            self.assertEqual(result["session_id"], "session-a")
            self.assertEqual(result["analysis_admitted_total"], 3)
            self.assertEqual(result["processed_deferred_total"], 3)
            self.assertEqual(result["exit_status"], 0)
            self.assertTrue(result["sidecar_path"].is_file())
            persisted = json.loads(result["sidecar_path"].read_text())
            self.assertEqual(persisted["measurement_run_id"], result["measurement_run_id"])
            self.assertEqual(
                persisted["measurement_instrument_version"],
                COLLECTOR.MEASUREMENT_INSTRUMENT_VERSION,
            )
            self.assertEqual(persisted["sessions"][0]["session_id"], "session-a")
            self.assertTrue(persisted["capture_complete"])
            serialized = result["sidecar_path"].read_text()
            self.assertNotIn("TRACEPRESS_SUBSCRIPTION_CANARY", serialized)
            self.assertEqual(stat.S_IMODE(result["sidecar_path"].stat().st_mode), 0o600)
            self.assertEqual(list(Path(directory).glob(".*.tmp")), [])

    def test_concurrent_out_of_order_sessions_never_cross_assign(self) -> None:
        from concurrent.futures import ThreadPoolExecutor

        cases = (("session-a", 3, 0.15), ("session-b", 7, 0.01), ("session-c", 12, 0.07))
        with TemporaryDirectory() as directory:
            output_dir = Path(directory)
            with ThreadPoolExecutor(max_workers=3) as executor:
                futures = [
                    executor.submit(
                        COLLECTOR.collect_scheduler_metrics,
                        [sys.executable, "-c", SIMULATOR, session, str(count), str(delay)],
                        output_dir,
                    )
                    for session, count, delay in cases
                ]
                results = [future.result() for future in futures]

            self.assertEqual(len({result["measurement_run_id"] for result in results}), 3)
            self.assertEqual(len({result["session_id"] for result in results}), 3)
            self.assertEqual(len({result["sidecar_path"] for result in results}), 3)
            self.assertTrue(all(result["capture_complete"] for result in results))
            observed = {result["session_id"]: result["analysis_admitted_total"] for result in results}
            self.assertEqual(observed, {"session-a": 3, "session-b": 7, "session-c": 12})
            self.assertEqual(
                [
                    result["session_id"]
                    for result in sorted(results, key=lambda result: result["finished_at"])
                ],
                ["session-b", "session-c", "session-a"],
            )
            self.assertTrue(
                all(
                    result["analysis_admitted_total"] == result["processed_deferred_total"]
                    + result["backlog_capacity_drops"]
                    for result in results
                )
            )

            merged = COLLECTOR.merge_sidecars([result["sidecar_path"] for result in results])
            self.assertEqual(len(merged["sessions"]), 3)
            self.assertEqual(
                {row["session_id"]: row["analysis_admitted_total"] for row in merged["sessions"]},
                observed,
            )

    def test_incomplete_capture_is_not_promoted_to_valid_sidecar(self) -> None:
        with TemporaryDirectory() as directory:
            result = COLLECTOR.collect_scheduler_metrics(
                [sys.executable, "-c", "print('not metadata')"],
                Path(directory),
            )

            self.assertFalse(result["capture_complete"])
            self.assertIn("missing_measurement_metadata", result["capture_reasons"])
            self.assertIn("missing_scheduler_metrics", result["capture_reasons"])

    def test_duplicate_context_counter_is_not_silently_overwritten(self) -> None:
        simulator = SIMULATOR + '\nprint("analysis_requests_seen=999", flush=True)\n'
        with TemporaryDirectory() as directory:
            result = COLLECTOR.collect_scheduler_metrics(
                [sys.executable, "-c", simulator, "session-duplicate", "3", "0"],
                Path(directory),
            )

            self.assertFalse(result["capture_complete"])
            self.assertIn(
                "duplicate_context_counters:analysis_requests_seen",
                result["capture_reasons"],
            )

    def test_merge_rejects_duplicate_run_identity(self) -> None:
        with TemporaryDirectory() as directory:
            path = COLLECTOR.collect_scheduler_metrics(
                [sys.executable, "-c", SIMULATOR, "session-a", "3", "0"],
                Path(directory),
            )["sidecar_path"]

            with self.assertRaisesRegex(ValueError, "duplicate or missing measurement_run_id"):
                COLLECTOR.merge_sidecars([path, path])

    def test_merge_rejects_tampered_root_and_incomplete_capture(self) -> None:
        with TemporaryDirectory() as directory:
            output_dir = Path(directory)
            path = COLLECTOR.collect_scheduler_metrics(
                [sys.executable, "-c", SIMULATOR, "session-a", "3", "0"],
                output_dir,
            )["sidecar_path"]
            document = json.loads(path.read_text())

            mismatched = output_dir / "mismatched.json"
            mismatched_document = json.loads(json.dumps(document))
            mismatched_document["session_id"] = "wrong-session"
            COLLECTOR.write_atomic_json(mismatched, mismatched_document)
            with self.assertRaisesRegex(ValueError, "root/row identity mismatch"):
                COLLECTOR.merge_sidecars([mismatched])

            incomplete = output_dir / "incomplete.json"
            incomplete_document = json.loads(json.dumps(document))
            incomplete_document["capture_complete"] = False
            incomplete_document["sessions"][0]["capture_complete"] = False
            COLLECTOR.write_atomic_json(incomplete, incomplete_document)
            with self.assertRaisesRegex(ValueError, "sidecar capture is incomplete"):
                COLLECTOR.merge_sidecars([incomplete])

    def test_capture_timeout_is_bounded_and_explicit(self) -> None:
        with TemporaryDirectory() as directory:
            result = COLLECTOR.collect_scheduler_metrics(
                [sys.executable, "-c", "import time; time.sleep(1)"],
                Path(directory),
                timeout_seconds=0.05,
            )

            self.assertFalse(result["capture_complete"])
            self.assertIn("capture_timeout", result["capture_reasons"])

    def test_capture_timeout_terminates_descendant_process_group(self) -> None:
        simulator = """
import subprocess, sys, time
marker = sys.argv[1]
subprocess.Popen([
    sys.executable,
    "-c",
    "import pathlib, sys, time; time.sleep(0.25); pathlib.Path(sys.argv[1]).write_text('alive')",
    marker,
])
time.sleep(2)
"""
        with TemporaryDirectory() as directory:
            marker = Path(directory) / "descendant-alive"
            result = COLLECTOR.collect_scheduler_metrics(
                [sys.executable, "-c", simulator, str(marker)],
                Path(directory),
                timeout_seconds=0.05,
            )

            self.assertFalse(result["capture_complete"])
            self.assertIn("capture_timeout", result["capture_reasons"])
            time.sleep(0.35)
            self.assertFalse(marker.exists())

    def test_oversized_machine_line_is_rejected(self) -> None:
        command = [
            sys.executable,
            "-c",
            "print('x' * 70000, flush=True)",
        ]
        with TemporaryDirectory() as directory:
            result = COLLECTOR.collect_scheduler_metrics(command, Path(directory), timeout_seconds=1)

            self.assertFalse(result["capture_complete"])
            self.assertIn("capture_line_too_large", result["capture_reasons"])


if __name__ == "__main__":
    raise SystemExit(unittest.main())
