#!/usr/bin/env python3
"""Contract tests for the offline baseline reports."""

from __future__ import annotations

import importlib.util
import json
import sqlite3
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[1]


def load_script(name: str):
    path = ROOT / "scripts" / name
    spec = importlib.util.spec_from_file_location(path.stem, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[path.stem] = module
    spec.loader.exec_module(module)
    return module


ANALYZER = load_script("analyze_baseline.py")
CONVERGENCE = load_script("check_baseline_convergence.py")
BENCHMARK = load_script("benchmark_phase3.py")


def create_fixture() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:")
    connection.executescript(
        """
        CREATE TABLE sessions (
            session_id TEXT PRIMARY KEY,
            state TEXT NOT NULL,
            ended_at TEXT
        );
        CREATE TABLE operations (
            operation_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            status TEXT NOT NULL
        );
        CREATE TABLE provider_requests (
            request_id TEXT PRIMARY KEY,
            operation_id TEXT NOT NULL,
            request_bytes INTEGER,
            request_kind TEXT NOT NULL,
            transport TEXT,
            model TEXT,
            reasoning_effort TEXT,
            parser_version INTEGER,
            observation_status TEXT,
            content_encoding TEXT,
            analysis_decode_status TEXT,
            wire_bytes INTEGER,
            decoded_bytes INTEGER
        );
        CREATE TABLE provider_attempts (
            attempt_id TEXT PRIMARY KEY,
            request_id TEXT NOT NULL,
            status_code INTEGER,
            status TEXT NOT NULL,
            observation_status TEXT,
            transport_error TEXT
        );
        CREATE TABLE provider_usage (
            attempt_id TEXT PRIMARY KEY,
            input_total INTEGER,
            input_cached INTEGER,
            output_total INTEGER,
            output_reasoning INTEGER,
            usage_status TEXT,
            normalizer_version INTEGER
        );
        CREATE TABLE context_snapshots (
            snapshot_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            provider_request_id TEXT NOT NULL,
            inference_operation_id TEXT NOT NULL,
            analysis_version INTEGER,
            status TEXT NOT NULL,
            completed_at_us INTEGER,
            recovered_at_us INTEGER,
            correlation_status TEXT,
            explicit_request_complete INTEGER
        );
        CREATE TABLE context_analysis_metrics (
            snapshot_id TEXT PRIMARY KEY,
            explicit_bytes INTEGER,
            unknown_block_count INTEGER,
            semantic_coverage_basis_points INTEGER,
            estimated_tool_definition_share REAL,
            estimated_tool_result_share REAL,
            stable_explicit_prefix_estimate INTEGER,
            estimator TEXT,
            estimator_version INTEGER,
            estimate_confidence TEXT
        );
        CREATE TABLE context_block_occurrences (
            block_occurrence_id TEXT PRIMARY KEY,
            snapshot_id TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            kind TEXT NOT NULL,
            role TEXT NOT NULL,
            origin TEXT NOT NULL,
            raw_bytes INTEGER NOT NULL,
            exact_fingerprint BLOB,
            semantic_fingerprint BLOB,
            fingerprint_version INTEGER,
            estimated_tokens INTEGER,
            estimator TEXT,
            estimator_version INTEGER,
            estimate_confidence TEXT,
            detected_kind TEXT,
            detector_version INTEGER,
            repetition_score REAL
        );
        CREATE TABLE context_deltas (
            current_snapshot_id TEXT PRIMARY KEY,
            previous_snapshot_id TEXT,
            repeated_blocks INTEGER,
            new_blocks INTEGER,
            changed_blocks INTEGER,
            removed_blocks INTEGER,
            repeated_estimated_tokens INTEGER,
            new_estimated_tokens INTEGER,
            common_prefix_estimated_tokens INTEGER
        );
        CREATE TABLE token_reconciliations (
            snapshot_id TEXT PRIMARY KEY,
            visible_estimated_tokens INTEGER,
            provider_input_tokens INTEGER,
            residual_tokens INTEGER,
            comparability TEXT NOT NULL
        );
        CREATE TABLE events (
            seq INTEGER PRIMARY KEY,
            event_type TEXT NOT NULL,
            payload BLOB NOT NULL
        );
        INSERT INTO sessions VALUES ('s1', 'closed', 'done');
        INSERT INTO operations VALUES ('op1', 's1', 'llm_inference', 'completed');
        INSERT INTO provider_requests VALUES
            ('r1', 'op1', 100, 'turn', 'chatgpt_codex_subscription', 'gpt-5.6-luna', 'xhigh', 1, 'complete', 'zstd', 'decoded', 100, 300);
        INSERT INTO provider_attempts VALUES ('a1', 'r1', 200, 'completed', 'complete', NULL);
        INSERT INTO provider_usage VALUES ('a1', 100, 60, 10, 4, 'final', 1);
        INSERT INTO context_snapshots VALUES ('snap1', 's1', 'r1', 'op1', 1, 'complete', 10, NULL, 'correlated', 1);
        INSERT INTO context_analysis_metrics VALUES ('snap1', 300, 1, 9000, 0.2, 0.6, 40, 'structural-heuristic', 1, 'heuristic');
        INSERT INTO context_block_occurrences VALUES
            ('b1', 'snap1', 0, 'text', 'user', 'human_authored', 100, X'01', X'11', 1, 10, 'structural-heuristic', 1, 'heuristic', 'plain_text', 1, 0.1),
            ('b2', 'snap1', 1, 'tool_result', 'tool', 'tool_generated', 200, X'02', NULL, 1, 30, 'structural-heuristic', 1, 'heuristic', 'json', 1, 0.4),
            ('b3', 'snap1', 2, 'unknown', 'unknown', 'unknown', 50, X'03', NULL, 1, NULL, NULL, NULL, NULL, NULL, NULL, 0.0);
        INSERT INTO token_reconciliations VALUES ('snap1', NULL, NULL, NULL, 'missing_local_estimate');
        INSERT INTO events VALUES (1, 'context.analysis.started', '{}');
        INSERT INTO events VALUES (2, 'context.analysis.completed', '{}');
        INSERT INTO events VALUES (3, 'provider.request.observed', '{}');
        INSERT INTO events VALUES (4, 'provider.response.completed', '{}');
        INSERT INTO events VALUES (5, 'provider.usage.observed', '{}');
        """
    )
    return connection


class BaselineAnalysisContractTests(unittest.TestCase):
    def test_benchmark_accepts_a_significant_forwarding_improvement(self) -> None:
        comparison = {
            "workloads": {
                "small_json": {
                    "dispatch_us": {
                        "available": True,
                        "delta_us": -500.0,
                        "bootstrap_median_delta_ci95": {
                            "lower_95_us": -600.0,
                            "upper_95_us": -400.0,
                        },
                    },
                    "proxy_ttfb_us": {"available": False},
                    "proxy_ttft_us": {"available": False},
                    "duration_us": {"available": False},
                }
            }
        }

        BENCHMARK.add_aa_envelope(
            comparison,
            {("small_json", "dispatch_us"): [{"absolute_us": 100.0, "relative": 0.1}]},
        )

        self.assertTrue(comparison["operationally_acceptable"])
        self.assertTrue(
            comparison["workloads"]["small_json"]["dispatch_us"]["operationally_acceptable"]
        )

    def test_benchmark_rejects_snapshot_identity_mismatch(self) -> None:
        measured = {
            "queue_observability": {
                "stdout_counters": {
                    "analysis_requests_seen": 2,
                    "analysis_requests_complete": 2,
                    "analysis_requests_partial": 0,
                    "analysis_requests_dropped": 0,
                },
                "durable": {
                    "phase2_provider_requests": 1,
                    "context_snapshots": 2,
                    "context_snapshot_request_ids": 2,
                    "context_snapshots_without_provider_request": 0,
                    "terminal_context_snapshots": 2,
                    "context_analysis_dropped_events": 0,
                    "context_analysis_event_count": 2,
                },
            }
        }

        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.validate_analysis_lifecycle(
                measured,
                phase="on",
                analysis_mode="shadow",
                workload="small_json",
                burst_width=1,
            )

    def test_report_is_token_weighted_and_keeps_missing_reconciliation_unknown(self) -> None:
        connection = create_fixture()
        connection.execute(
            "INSERT INTO events VALUES (?, ?, ?)",
            (6, "context.analysis.dropped", '{"reason":"observer_backpressure","dropped_count":2}'),
        )
        report = ANALYZER.analyze_connection(
            connection,
            measurement_id="baseline-001",
            cohort_label="n20",
            cohort_kind="naturalistic",
            tracepress_commit="70957bba",
            codex_version="0.154.0",
        )

        self.assertEqual(report["dataset"]["sessions_total"], 1)
        self.assertEqual(report["dataset"]["requests_total"], 1)
        self.assertEqual(report["quality"]["analysis_requests_seen"], 1)
        self.assertEqual(report["quality"]["analysis_complete"], 1)
        self.assertEqual(report["quality"]["analysis_dropped"], 0)
        self.assertEqual(report["quality"]["analysis_coverage"], 1.0)
        self.assertEqual(report["quality"]["analysis_auxiliary_drop_events"], 1)
        self.assertEqual(report["quality"]["analysis_auxiliary_drop_work"], 2)
        self.assertEqual(report["quality"]["analysis_unmatched_events"], 1)
        self.assertEqual(report["analysis_integrity"]["measurement_integrity"], "failed")
        self.assertEqual(report["analysis_integrity"]["request_coverage"], 1.0)
        self.assertEqual(report["request_analysis_ledger"][0]["outcome"], "Complete")
        self.assertEqual(report["quality"]["correlation_coverage"], 1.0)
        self.assertEqual(report["quality"]["event_drop_work"]["observer_backpressure"], 2)
        self.assertEqual(report["quality"]["semantic_coverage"]["mean"], 0.9)
        self.assertFalse(report["provider_usage"]["token_reconciliation_available"])
        self.assertEqual(report["provider_usage"]["reconciliation_unavailable"]["missing_local_estimate"], 1)

        content = {row["name"]: row for row in report["composition"]["detected_content"]}
        self.assertGreater(content["json"]["token_share"], content["plain_text"]["token_share"])
        self.assertEqual(report["unknown"]["estimated_tokens"], 0)
        self.assertEqual(report["unknown"]["block_count"], 1)
        coverage = report["quality"]["token_estimation_coverage"]
        detected_coverage = {row["name"]: row for row in coverage["by_detected_content_kind"]}
        self.assertEqual(detected_coverage["plain_text"]["block_coverage"], 1.0)
        self.assertEqual(detected_coverage["json"]["block_coverage"], 1.0)
        self.assertEqual(detected_coverage["unknown"]["block_coverage"], 0.0)
        self.assertEqual(
            report["composition"]["by_origin_kind_detected_content"][0]["name"],
            "origin=tool_generated | kind=tool_result | detected_kind=json",
        )
        self.assertEqual(
            report["repetition_by_origin_kind_detected_content"][0]["name"],
            "origin=tool_generated | kind=tool_result | detected_kind=json",
        )

    def test_report_can_restrict_accounting_to_selected_database_sessions(self) -> None:
        connection = create_fixture()
        connection.executescript(
            """
            INSERT INTO sessions VALUES ('s2', 'closed', 'done');
            INSERT INTO operations VALUES ('op2', 's2', 'llm_inference', 'completed');
            INSERT INTO provider_requests VALUES
                ('r2', 'op2', 200, 'turn', 'chatgpt_codex_subscription', 'gpt-5.6-luna', 'xhigh', 1, 'complete', 'zstd', 'decoded', 200, 600);
            """
        )

        report = ANALYZER.analyze_connection(
            connection,
            measurement_id="baseline-002",
            cohort_label="n10",
            cohort_kind="naturalistic",
            tracepress_commit="a15ac2d",
            codex_version="0.154.0",
            session_ids={"s1"},
        )

        self.assertEqual(report["dataset"]["sessions_total"], 1)
        self.assertEqual(report["dataset"]["requests_total"], 1)
        self.assertEqual(report["dataset"]["session_filter"]["excluded_sessions"], 1)
        self.assertEqual(report["analysis_integrity"]["eligible_requests"], 1)
        self.assertEqual(report["analysis_integrity"]["measurement_integrity"], "passed")

    def test_ledger_partitions_eligible_requests_and_attaches_identified_drop(self) -> None:
        connection = create_fixture()
        connection.executescript(
            """
            INSERT INTO operations VALUES ('op2', 's1', 'llm_inference', 'completed');
            INSERT INTO provider_requests VALUES
                ('r2', 'op2', 80, 'turn', 'chatgpt_codex_subscription', 'gpt-5.6-luna', 'xhigh', 1, 'complete', 'zstd', 'decoded', 80, 240);
            INSERT INTO provider_requests VALUES
                ('r3', 'op2', 70, 'turn', 'chatgpt_codex_subscription', 'gpt-5.6-luna', 'xhigh', 1, 'partial', 'zstd', 'decoded', 70, 210);
            INSERT INTO context_snapshots VALUES ('snap2', 's1', 'r2', 'op2', 1, 'partial', 11, NULL, 'degraded', 1);
            INSERT INTO events VALUES
                (7, 'context.analysis.dropped', '{"request_id":"r3","forward_id":"f3","reason":"deferred_backlog_capacity","dropped_count":1,"terminal":true}');
            """
        )

        report = ANALYZER.analyze_connection(
            connection,
            measurement_id="baseline-001",
            cohort_label="ledger",
            cohort_kind="naturalistic",
            tracepress_commit="70957bba",
            codex_version="0.154.0",
        )

        integrity = report["analysis_integrity"]
        self.assertEqual(integrity["eligible_requests"], 3)
        self.assertEqual(integrity["complete_requests"], 1)
        self.assertEqual(integrity["partial_requests"], 1)
        self.assertEqual(integrity["dropped_requests"], 1)
        self.assertEqual(integrity["auxiliary_drop_events"], 1)
        self.assertEqual(integrity["unmatched_events"], 0)
        self.assertEqual(integrity["analysis_outcome_conflicts"], 0)
        self.assertTrue(integrity["outcome_partition_valid"])

        ledger = {row["provider_request_id"]: row for row in report["request_analysis_ledger"]}
        self.assertEqual(ledger["r1"]["outcome"], "Complete")
        self.assertEqual(ledger["r2"]["outcome"], "Partial")
        self.assertEqual(ledger["r3"]["outcome"], "Dropped")
        self.assertEqual(ledger["r3"]["forward_id"], "f3")
        self.assertEqual(ledger["r3"]["durable_drop_events"], 1)
        self.assertEqual(ledger["r3"]["request_bytes"], 70)

    def test_ledger_rejects_complete_snapshot_with_terminal_drop(self) -> None:
        connection = create_fixture()
        connection.execute(
            "INSERT INTO events VALUES (?, ?, ?)",
            (7, "context.analysis.dropped", '{"request_id":"r1","reason":"cancelled","terminal":true}'),
        )

        report = ANALYZER.analyze_connection(
            connection,
            measurement_id="baseline-001",
            cohort_label="conflict",
            cohort_kind="naturalistic",
            tracepress_commit="70957bb",
            codex_version="0.154.0",
        )

        self.assertEqual(report["request_analysis_ledger"][0]["outcome"], "Complete")
        self.assertEqual(report["analysis_integrity"]["analysis_outcome_conflicts"], 1)
        self.assertEqual(report["analysis_integrity"]["measurement_integrity"], "failed")

    def test_recovered_partial_snapshot_is_partial_not_dropped(self) -> None:
        connection = create_fixture()
        connection.execute(
            "UPDATE context_snapshots SET status = 'partial', completed_at_us = NULL, recovered_at_us = 99 WHERE snapshot_id = 'snap1'"
        )

        report = ANALYZER.analyze_connection(
            connection,
            measurement_id="baseline-001",
            cohort_label="recovered",
            cohort_kind="naturalistic",
            tracepress_commit="70957bba",
            codex_version="0.154.0",
        )

        self.assertEqual(report["request_analysis_ledger"][0]["outcome"], "Partial")
        self.assertEqual(report["quality"]["analysis_complete"], 0)
        self.assertEqual(report["quality"]["analysis_partial"], 1)
        self.assertEqual(report["quality"]["analysis_dropped"], 0)

    def test_compaction_transport_is_ineligible_without_context_snapshot(self) -> None:
        ledger, integrity = ANALYZER.request_analysis_ledger(
            [
                {
                    "request_id": "compact-1",
                    "operation_id": "op1",
                    "request_kind": "compaction_v2",
                    "method": "POST",
                    "route": "/v1/responses/compact",
                }
            ],
            [],
            [],
            {"op1": "s1"},
        )

        self.assertEqual(len(ledger), 1)
        self.assertFalse(ledger[0]["eligible"])
        self.assertEqual(ledger[0]["outcome"], "Ineligible")
        self.assertEqual(integrity["eligible_requests"], 0)
        self.assertEqual(integrity["measurement_integrity"], "passed")

    def test_compaction_report_pairs_adjacent_visible_windows_without_payloads(self) -> None:
        requests = [
            {"request_id": "r1", "request_kind": "turn"},
            {"request_id": "r2", "request_kind": "compaction_v2", "compaction_trigger": "unknown"},
            {"request_id": "r3", "request_kind": "turn"},
        ]
        attempts = {
            "r2": [{"compaction_output_seen": 1}],
        }
        usage = {"r1": 100, "r2": 120, "r3": 40}
        snapshots = [
            {"snapshot_id": "s1", "session_id": "session", "provider_request_id": "r1"},
            {"snapshot_id": "s2", "session_id": "session", "provider_request_id": "r2"},
            {"snapshot_id": "s3", "session_id": "session", "provider_request_id": "r3"},
        ]
        context = {
            "s1": {"session_id": "session", "snapshot_order": 0, "request_id": "r1"},
            "s2": {"session_id": "session", "snapshot_order": 1, "request_id": "r2"},
            "s3": {"session_id": "session", "snapshot_order": 2, "request_id": "r3"},
        }
        blocks = {
            "s1": [{"kind": "tool_result", "detected_kind": "json", "estimated_tokens": 20, "raw_bytes": 80, "semantic_fingerprint": b"same"}],
            "s2": [],
            "s3": [{"kind": "tool_result", "detected_kind": "json", "estimated_tokens": 10, "raw_bytes": 40, "semantic_fingerprint": b"same"}],
        }

        result = ANALYZER.compaction_report(
            requests,
            attempts,
            usage,
            snapshots,
            blocks,
            context,
            "compaction_calibration",
        )

        self.assertEqual(result["requests"], 1)
        self.assertEqual(result["output_seen"], 1)
        self.assertEqual(result["pre_post_pairs"], 1)
        self.assertEqual(result["observed_context_reduction"]["count"], 1)
        self.assertEqual(result["observed_context_reduction"]["p50"], 0.5)
        self.assertEqual(result["survival_by_category"][0]["survival_share"], 1.0)
        self.assertEqual(len(result["windows"]), 2)

    def test_report_contains_no_payload_or_fingerprint_values(self) -> None:
        report = ANALYZER.analyze_connection(
            create_fixture(),
            measurement_id="baseline-001",
            cohort_label="n20",
            cohort_kind="naturalistic",
            tracepress_commit="70957bba",
            codex_version="0.154.0",
        )
        serialized = json.dumps(report, sort_keys=True)
        self.assertNotIn("0101", serialized)
        self.assertNotIn("0203", serialized)
        self.assertNotIn("Authorization", serialized)
        self.assertNotIn("Bearer", serialized)

    def test_artifacts_include_a_reproducible_manifest(self) -> None:
        report = ANALYZER.analyze_connection(
            create_fixture(),
            measurement_id="baseline-001",
            cohort_label="n20",
            cohort_kind="naturalistic",
            tracepress_commit="70957bba",
            codex_version="0.154.0",
        )
        with TemporaryDirectory() as directory:
            paths = ANALYZER.write_report_artifacts(report, Path(directory))
            manifest = json.loads(paths[2].read_text())

        self.assertEqual(manifest["measurement_id"], "baseline-001")
        self.assertEqual(manifest["cohort_kind"], "naturalistic")
        self.assertEqual(manifest["tracepress_commit"], "70957bba")
        self.assertEqual(manifest["analysis_version"], 1)

    def test_report_aggregates_scheduler_metrics_from_metadata_sidecar(self) -> None:
        report = ANALYZER.analyze_connection(
            create_fixture(),
            measurement_id="baseline-002",
            cohort_label="n10",
            cohort_kind="naturalistic",
            tracepress_commit="a15ac2d",
            codex_version="0.154.0",
            runtime_metrics={
                "sessions": [
                    {
                        "session_id": "s1",
                        "workload": "repo_exploration",
                        "concurrency_mode": "serial",
                        "high_water_items": 2,
                        "high_water_bytes": 176717,
                        "analysis_wait_us": 666,
                        "deferred_total": 1,
                        "processed_deferred_total": 1,
                        "backlog_capacity_drops": 0,
                    }
                ]
            },
        )

        scheduler = report["scheduler"]
        self.assertEqual(scheduler["sessions_with_metrics"], 1)
        self.assertEqual(scheduler["deferred_total"], 1)
        self.assertEqual(scheduler["processed_deferred_total"], 1)
        self.assertEqual(scheduler["backlog_capacity_drops"], 0)
        self.assertEqual(scheduler["analysis_deferral_rate"], 1.0)
        self.assertEqual(scheduler["analysis_loss_rate"], 0.0)
        self.assertEqual(scheduler["high_water_items"]["p90"], 2.0)
        self.assertEqual(scheduler["analysis_wait_us"]["p50"], 666.0)
        self.assertEqual(report["missingness"]["by_workload"][0]["name"], "repo_exploration")
        self.assertEqual(report["missingness"]["unavailable_dimensions"], [])


class BaselineConvergenceContractTests(unittest.TestCase):
    @staticmethod
    def report(n: int, share: float = 0.5) -> dict:
        return {
            "dataset": {"sessions_total": n},
            "quality": {
                "analysis_coverage": 1.0,
                "correlation_coverage": 1.0,
                "forwarding_errors": 0,
                "context_malformed": 0,
                "measurement_integrity": "passed",
            },
            "composition": {
                "detected_content": [
                    {"name": "plain_text", "token_share": share},
                    {"name": "json", "token_share": 1.0 - share},
                ],
                "by_kind": [],
            },
            "repetition": {"exact_repeated_token_share": 0.1, "semantic_repeated_token_share": 0.2},
            "provider_usage": {"cache_ratio": 0.4},
            "stable_prefix": {"share": 0.3},
            "distributions": {"estimated_tokens": {"p50": 10.0, "p90": 20.0}},
            "opportunity_ranking": [{"name": "json"}, {"name": "plain_text"}, {"name": "tool_result"}],
        }

    def test_convergence_requires_two_stable_transitions_and_n40(self) -> None:
        n20 = self.report(20)
        n30 = self.report(30)
        n40 = self.report(40)
        self.assertEqual(CONVERGENCE.evaluate_reports([n20, n30])["status"], "NEED_MORE_SESSIONS")
        result = CONVERGENCE.evaluate_reports([n20, n30, n40])
        self.assertEqual(result["status"], "CONVERGED")

    def test_convergence_rejects_large_share_change(self) -> None:
        result = CONVERGENCE.evaluate_reports([self.report(20), self.report(30, 0.7), self.report(40, 0.7)])
        self.assertEqual(result["status"], "NEED_MORE_SESSIONS")

    def test_convergence_rejects_failed_measurement_integrity(self) -> None:
        reports = [self.report(20), self.report(30), self.report(40)]
        reports[-1]["analysis_integrity"] = {"measurement_integrity": "failed"}
        result = CONVERGENCE.evaluate_reports(reports)
        self.assertEqual(result["status"], "NEED_MORE_SESSIONS")

    def test_convergence_rejects_mixed_measurement_manifests(self) -> None:
        reports = [self.report(20), self.report(30), self.report(40)]
        for report in reports:
            report["measurement_id"] = "baseline-002"
            report["manifest"] = {"measurement_instrument_version": 2}
        reports[-1]["manifest"] = {"measurement_instrument_version": 3}
        result = CONVERGENCE.evaluate_reports(reports)
        self.assertEqual(result["status"], "NEED_MORE_SESSIONS")
        self.assertIn("reports mix measurement manifests or instrument versions", result["reasons"])

    def test_convergence_rejects_stratified_missingness_bias(self) -> None:
        reports = [self.report(20), self.report(30), self.report(40)]
        for report in reports:
            report["missingness"] = {
                "by_request_size_quartile": [
                    {"name": "q4", "eligible_requests": 10, "drop_rate": 0.1}
                ]
            }
        result = CONVERGENCE.evaluate_reports(reports)
        self.assertEqual(result["status"], "NEED_MORE_SESSIONS")
        self.assertIn("missingness is above 5% in an important request stratum", result["reasons"])

    def test_convergence_rejects_scheduler_loss_or_missing_sidecar_sessions(self) -> None:
        reports = [self.report(20), self.report(30), self.report(40)]
        for report in reports:
            report["scheduler"] = {
                "available": True,
                "sessions_with_metrics": report["dataset"]["sessions_total"],
                "analysis_loss_rate": 0.0,
                "backlog_capacity_drops": 0,
            }
        reports[-1]["scheduler"]["analysis_loss_rate"] = 0.02
        result = CONVERGENCE.evaluate_reports(reports)
        self.assertEqual(result["status"], "NEED_MORE_SESSIONS")
        self.assertIn("one or more measurement quality gates failed", result["reasons"])

        reports[-1]["scheduler"]["analysis_loss_rate"] = 0.0
        reports[-1]["scheduler"]["sessions_with_metrics"] = 39
        result = CONVERGENCE.evaluate_reports(reports)
        self.assertEqual(result["status"], "NEED_MORE_SESSIONS")


if __name__ == "__main__":
    raise SystemExit(unittest.main())
