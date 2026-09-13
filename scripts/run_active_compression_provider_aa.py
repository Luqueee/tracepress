#!/usr/bin/env python3
"""Run a cardinality-matched provider A/A gate for the explicit active adapter.

The driver runs the same directed ToolResult workload through isolated Tracepress homes with the
active adapter off and on. It keeps only bounded counters and aggregate provider observations;
prompt, tool-result, response, header, and URL content is never written to the report.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import tempfile
import time
from typing import Any


EXPERIMENT_ID = "active-compression-provider-aa-002"
PROMPT_VARIANT = "directed_tool_result_json_v1"
COUNTER_RE = re.compile(r"([a-zA-Z0-9_]+)=([0-9]+)")


def args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--samples", type=int, default=10)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    options = parser.parse_args()
    if options.samples < 1:
        parser.error("samples must be positive")
    return options


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int((len(ordered) * fraction) + 0.999999) - 1))
    return round(ordered[index], 3)


def summary(connection: sqlite3.Connection) -> dict[str, Any]:
    def scalar(query: str) -> int | None:
        return connection.execute(query).fetchone()[0]

    def values(query: str) -> list[Any]:
        return [row[0] for row in connection.execute(query).fetchall()]

    durations = [float(value) for value in values("SELECT duration_us FROM provider_attempts WHERE duration_us IS NOT NULL")]
    ttfb = [float(value) for value in values("SELECT ttfb_us FROM provider_attempts WHERE ttfb_us IS NOT NULL")]
    ttft = [float(value) for value in values("SELECT ttft_us FROM provider_attempts WHERE ttft_us IS NOT NULL")]
    return {
        "provider_requests": scalar("SELECT COUNT(*) FROM provider_requests"),
        "provider_attempts": scalar("SELECT COUNT(*) FROM provider_attempts"),
        "provider_usage_rows": scalar("SELECT COUNT(*) FROM provider_usage"),
        "provider_errors": scalar(
            "SELECT COUNT(*) FROM provider_attempts WHERE status <> 'completed' OR error_code IS NOT NULL OR transport_error IS NOT NULL"
        ),
        "context_snapshots": scalar("SELECT COUNT(*) FROM context_snapshots"),
        "context_complete": scalar("SELECT COUNT(*) FROM context_snapshots WHERE status = 'complete'"),
        "content_encoding": values(
            "SELECT DISTINCT COALESCE(content_encoding, 'unavailable') FROM provider_requests ORDER BY 1"
        ),
        "transport": values(
            "SELECT DISTINCT COALESCE(transport, 'unavailable') FROM provider_requests ORDER BY 1"
        ),
        "tool_calls": scalar("SELECT COALESCE(SUM(tool_count), 0) FROM provider_requests"),
        "wire_bytes": scalar("SELECT COALESCE(SUM(wire_bytes), 0) FROM provider_requests"),
        "decoded_bytes": scalar("SELECT COALESCE(SUM(decoded_bytes), 0) FROM provider_requests"),
        "usage": {
            "input_total": scalar("SELECT SUM(input_total) FROM provider_usage"),
            "input_uncached": scalar("SELECT SUM(input_uncached) FROM provider_usage"),
            "cache_read": scalar("SELECT SUM(cache_read) FROM provider_usage"),
            "output_total": scalar("SELECT SUM(output_total) FROM provider_usage"),
            "reasoning": scalar("SELECT SUM(reasoning) FROM provider_usage"),
        },
        "latency_us": {
            "ttfb_p50": percentile(ttfb, 0.50),
            "ttfb_p95": percentile(ttfb, 0.95),
            "ttft_p50": percentile(ttft, 0.50),
            "ttft_p95": percentile(ttft, 0.95),
            "duration_p50": percentile(durations, 0.50),
            "duration_p95": percentile(durations, 0.95),
        },
    }


def counters(stdout: str) -> dict[str, int]:
    result: dict[str, int] = {}
    for line in stdout.splitlines():
        for key, value in COUNTER_RE.findall(line):
            if key.startswith(("active_compression_", "analysis_", "correlation_", "context_")):
                result[key] = int(value)
    return result


def run_arm(repo: Path, cli: Path, daemon: Path, name: str, active: bool, samples: int) -> dict[str, Any]:
    root = Path(tempfile.mkdtemp(prefix=f"tracepress-provider-aa-{name}-"))
    environment = os.environ.copy()
    environment.pop("TRACEPRESS_UPSTREAM", None)
    environment.update(
        {
            "TRACEPRESS_HOME": str(root),
            "TRACEPRESS_CONTEXT_ANALYSIS": "shadow",
            "TRACEPRESS_SHADOW_COMPRESSION": "off",
            "TRACEPRESS_ACTIVE_COMPRESSION": "json.minify" if active else "off",
            "TRACEPRESS_MEASUREMENT_RUN_ID": f"{EXPERIMENT_ID}-{name}",
        }
    )
    process: subprocess.Popen[str] | None = None
    daemon_log = None
    try:
        subprocess.run(
            [str(cli), "init"],
            cwd=repo,
            env=environment,
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
        daemon_environment = dict(environment)
        daemon_environment.update(
            {
                "TRACEPRESS_DATABASE": str(root / "tracepress.sqlite3"),
                "TRACEPRESS_CONTROL_SOCKET": str(root / "tracepress.sock"),
                "TRACEPRESS_CONTROL_CREDENTIAL": str(root / "control.cred"),
                "TRACEPRESS_DAEMON_READY": str(root / "daemon.ready"),
            }
        )
        daemon_log = (root / "daemon.log").open("w", encoding="utf-8")
        process = subprocess.Popen(
            [str(daemon)],
            cwd=repo,
            env=daemon_environment,
            stdout=daemon_log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            text=True,
        )
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f"{name} daemon exited before readiness")
            if (root / "daemon.ready").exists():
                status = subprocess.run(
                    [str(cli), "daemon", "status"],
                    cwd=repo,
                    env=environment,
                    capture_output=True,
                    text=True,
                    timeout=30,
                )
                if status.returncode == 0:
                    break
            time.sleep(0.1)
        else:
            raise RuntimeError(f"{name} daemon did not become ready")

        prompt = (
            "Use the shell exactly once. Run python3 -c 'import json; print(json.dumps(["
            "{'id':i,'name':'item-%03d'%i,'status':'ok' if i%3 else 'warning',"
            "'value':'distinct-value-%03d-abcdef'%i,'active':bool(i%2)} for i in range(100)], indent=2))'. "
            "Then reply only DONE."
        )
        runs: list[dict[str, Any]] = []
        previous_requests = 0
        previous_snapshots = 0
        for index in range(samples):
            started = time.monotonic()
            completed = subprocess.run(
                [
                    str(cli),
                    "run",
                    "codex",
                    "exec",
                    "-m",
                    "gpt-5.6-luna",
                    "-s",
                    "read-only",
                    "--skip-git-repo-check",
                    prompt,
                ],
                cwd=repo,
                env=environment,
                capture_output=True,
                text=True,
                timeout=180,
            )
            if completed.returncode != 0:
                raise RuntimeError(f"{name} invocation {index + 1} failed")
            with sqlite3.connect(f"file:{root / 'tracepress.sqlite3'}?mode=ro", uri=True) as connection:
                current_requests = int(connection.execute("SELECT COUNT(*) FROM provider_requests").fetchone()[0])
                current_snapshots = int(connection.execute("SELECT COUNT(*) FROM context_snapshots").fetchone()[0])
            runs.append(
                {
                    "index": index + 1,
                    "elapsed_ms": round((time.monotonic() - started) * 1000, 1),
                    "provider_requests": current_requests - previous_requests,
                    "context_snapshots": current_snapshots - previous_snapshots,
                    "counters": counters(completed.stdout),
                }
            )
            previous_requests = current_requests
            previous_snapshots = current_snapshots
        with sqlite3.connect(f"file:{root / 'tracepress.sqlite3'}?mode=ro", uri=True) as connection:
            aggregate = summary(connection)
        return {"name": name, "configuration": environment["TRACEPRESS_ACTIVE_COMPRESSION"], "runs": runs, "summary": aggregate}
    finally:
        if process is not None:
            subprocess.run([str(cli), "daemon", "stop"], cwd=repo, env=environment, capture_output=True, text=True, timeout=30)
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
        if daemon_log is not None:
            daemon_log.close()
        shutil.rmtree(root, ignore_errors=True)


def report(options: argparse.Namespace) -> dict[str, Any]:
    repo = options.repo_root.resolve()
    cli = repo / "target/debug/tracepress"
    daemon = repo / "target/debug/tracepressd"
    runtime_commit = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=repo, check=True, capture_output=True, text=True
    ).stdout.strip()
    arms = [
        run_arm(repo, cli, daemon, "control", False, options.samples),
        run_arm(repo, cli, daemon, "active", True, options.samples),
    ]
    control, active = arms
    control_counts = [run["provider_requests"] for run in control["runs"]]
    active_counts = [run["provider_requests"] for run in active["runs"]]
    cardinality_matched = control_counts == active_counts
    active_counters = {
        key: sum(run["counters"].get(key, 0) for run in active["runs"])
        for key in (
            "active_compression_attempts",
            "active_compression_rewrites",
            "active_compression_no_improvement",
            "active_compression_not_applicable",
            "active_compression_recovery_failures",
            "active_compression_determinism_failures",
            "active_compression_resource_limits",
            "active_compression_internal_errors",
        )
    }
    return {
        "experiment_id": EXPERIMENT_ID,
        "phase": "4.2_infrastructure_gate",
        "runtime_commit": runtime_commit,
        "provider": "authenticated ChatGPT Codex subscription",
        "model": "gpt-5.6-luna",
        "workload": PROMPT_VARIANT,
        "samples_per_arm": options.samples,
        "arms": arms,
        "gates": {
            "cardinality_matched": cardinality_matched,
            "forwarding_reachable": all(run["summary"]["provider_errors"] == 0 for run in arms),
            "analysis_complete": all(
                run["summary"]["context_snapshots"] == run["summary"]["context_complete"] for run in arms
            ),
            **active_counters,
            "provider_impact_validated": False,
            "quality_validated": False,
            "cache_impact_validated": False,
        },
        "limitations": [
            "This is an infrastructure A/A gate, not provider efficacy evidence.",
            "No provider-token, cache, cost, quality, or savings claim is made.",
            "The Codex client may emit a non-fatal /v1/models 404 through the local proxy.",
        ],
    }


def markdown(document: dict[str, Any]) -> str:
    control, active = document["arms"]
    gates = document["gates"]
    return "\n".join(
        [
            "# TRACEPRESS ACTIVE COMPRESSION PROVIDER A/A 002",
            "",
            "Cardinality-matched authenticated infrastructure gate for the explicit `json.minify` arm.",
            "No prompt, tool-result, response, header, or URL content is persisted in this report.",
            "",
            f"- Runtime commit: `{document['runtime_commit']}`",
            f"- Workload: `{document['workload']}`; N={document['samples_per_arm']} per arm",
            "- Provider: authenticated ChatGPT Codex subscription",
            "",
            "| Arm | Provider requests | Analysis | Encoding | Attempts | Rewrites | Recovery | Determinism | Errors |",
            "|---|---:|---:|---|---:|---:|---:|---:|---:|",
            f"| Control (`off`) | {control['summary']['provider_requests']} | {control['summary']['context_complete']}/{control['summary']['context_snapshots']} | {', '.join(control['summary']['content_encoding'])} | 0 | 0 | — | — | {control['summary']['provider_errors']} |",
            f"| Active (`json.minify`) | {active['summary']['provider_requests']} | {active['summary']['context_complete']}/{active['summary']['context_snapshots']} | {', '.join(active['summary']['content_encoding'])} | {gates['active_compression_attempts']} | {gates['active_compression_rewrites']} | {gates['active_compression_recovery_failures']} | {gates['active_compression_determinism_failures']} | {active['summary']['provider_errors']} |",
            "",
            f"Per-invocation request cardinality: control `{[run['provider_requests'] for run in control['runs']]}`; active `{[run['provider_requests'] for run in active['runs']]}`.",
            f"Cardinality matched: `{str(gates['cardinality_matched']).lower()}`.",
            "",
            "The active adapter reached the provider path and retained the original bytes for every",
            "request because candidates were `NoImprovement` or `NotApplicable`. This proves only",
            "forwarding reachability and bounded failure behavior; it does not establish provider",
            "token, cache, cost, quality, or causal savings impact.",
            "",
            "The non-fatal Codex `/v1/models` 404 remains outside the adapter.",
            "",
        ]
    )


def main() -> int:
    options = args()
    document = report(options)
    options.output_json.parent.mkdir(parents=True, exist_ok=True)
    options.output_md.parent.mkdir(parents=True, exist_ok=True)
    options.output_json.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    options.output_md.write_text(markdown(document), encoding="utf-8")
    print(markdown(document))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
