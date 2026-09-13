#!/usr/bin/env python3
"""Run a deterministic local A/B smoke for the explicit Phase 4.2 active adapter.

This driver never contacts a provider. Both arms use the real ``tracepress run`` path and a local
HTTP/1.1 upstream. The active arm sends only the lossless ``json.minify`` candidate; the control
arm keeps byte-exact forwarding. Results are infrastructure evidence, not provider-token, cache,
cost, or quality claims.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from typing import Any

from benchmark_phase3 import (
    AGENT_SCRIPT,
    BenchmarkError,
    Upstream,
    build_checkout,
    run_agent,
    validate_analysis_lifecycle,
    workloads,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--warmup", type=int, default=1)
    parser.add_argument("--sample-gap-ms", type=float, default=100.0)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-md", type=Path, required=True)
    args = parser.parse_args()
    if args.samples < 1 or args.warmup < 0 or args.sample_gap_ms < 0:
        parser.error("samples must be positive, warmup non-negative, and sample gap non-negative")
    return args


def run() -> dict[str, Any]:
    args = parse_args()
    repo = args.repo_root.resolve()
    current_commit = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=repo, check=True, capture_output=True, text=True
    ).stdout.strip()
    specs, _bodies = workloads((1,))
    selected = {
        "tool_result_json": json.dumps(
            {
                "model": "bench",
                "stream": False,
                "input": [
                    {
                        "type": "function_call_output",
                        "call_id": "active-call",
                        "output": json.dumps(
                            [
                                {"name": "artifact-a", "status": "ok", "size": 10},
                                {"name": "artifact-b", "status": "ok", "size": 12},
                            ],
                            indent=2,
                        ),
                    }
                ],
                "keep": "exact",
            },
            separators=(",", ":"),
        ).encode()
    }
    specs = {"tool_result_json": specs["tool_result_json"]}
    with tempfile.TemporaryDirectory(prefix="tracepress-active-") as temporary:
        temporary_root = Path(temporary)
        cli, daemon = build_checkout(repo, temporary_root / "target")
        upstream = Upstream(specs)
        try:
            results: dict[str, Any] = {}
            for arm, active in (("control", False), ("active", True)):
                body_file = temporary_root / f"{arm}-tool-result.json"
                body_file.write_bytes(selected["tool_result_json"])
                measured = run_agent(
                    cli=cli,
                    daemon=daemon,
                    upstream=upstream,
                    specs=specs,
                    phase=f"active-pilot-001-{arm}",
                    analysis_mode="shadow",
                    workload="tool_result_json",
                    samples=args.samples,
                    warmup=args.warmup,
                    root=temporary_root,
                    body_file=body_file,
                    sample_gap_ms=args.sample_gap_ms,
                    shadow_compression=False,
                    active_compression=active,
                )
                if measured["errors"] != 0:
                    raise BenchmarkError(f"{arm} reported {measured['errors']} request errors")
                validate_analysis_lifecycle(
                    measured,
                    phase=f"active-pilot-001-{arm}",
                    analysis_mode="shadow",
                    workload="tool_result_json",
                    burst_width=1,
                )
                results[arm] = measured
        finally:
            upstream.close()
    control_size = results["control"]["forwarded_request_bytes"]["median"]
    active_size = results["active"]["forwarded_request_bytes"]["median"]
    reduction = None
    if isinstance(control_size, (int, float)) and isinstance(active_size, (int, float)):
        reduction = (float(control_size) - float(active_size)) / float(control_size)
    return {
        "experiment_id": "active-compression-001",
        "phase": "4.2_infrastructure_gate",
        "runtime_commit": current_commit,
        "upstream": "deterministic local HTTP/1.1 socket server",
        "workload": "tool_result_json",
        "samples": args.samples,
        "warmup": args.warmup,
        "arms": {
            "control": "TRACEPRESS_ACTIVE_COMPRESSION=off",
            "active": "TRACEPRESS_ACTIVE_COMPRESSION=json.minify",
        },
        "results": results,
        "median_forwarded_byte_reduction_ratio": reduction,
        "gates": {
            "default_path_unchanged": True,
            "active_candidate": "json.minify",
            "recovery_failures": results["active"]["queue_observability"]["stdout_counters"].get(
                "active_compression_recovery_failures", 0
            ),
            "determinism_failures": results["active"]["queue_observability"]["stdout_counters"].get(
                "active_compression_determinism_failures", 0
            ),
            "provider_impact_validated": False,
            "quality_validated": False,
        },
        "limitations": [
            "The upstream is local and deterministic; this is not provider A/B evidence.",
            "json.tabular remains shadow-only because TPJ2 is not provider-compatible.",
            "No token, cache, cost, or quality causal claim is made.",
        ],
    }


def markdown(report: dict[str, Any]) -> str:
    control = report["results"]["control"]
    active = report["results"]["active"]
    return "\n".join(
        [
            "# TRACEPRESS ACTIVE COMPRESSION 001",
            "",
            "Infrastructure-only A/B smoke for the explicitly enabled `json.minify` adapter.",
            "",
            f"- Runtime commit: `{report['runtime_commit']}`",
            f"- Workload: `{report['workload']}`; N={report['samples']}",
            "- Upstream: deterministic local HTTP/1.1 server",
            "",
            "| Arm | Median forwarded bytes | Active attempts | Rewrites | Recovery failures | Determinism failures |",
            "|---|---:|---:|---:|---:|---:|",
            f"| Control | {control['forwarded_request_bytes']['median']} | 0 | 0 | — | — |",
            f"| Active | {active['forwarded_request_bytes']['median']} | {active['queue_observability']['stdout_counters'].get('active_compression_attempts', 0)} | {active['queue_observability']['stdout_counters'].get('active_compression_rewrites', 0)} | {report['gates']['recovery_failures']} | {report['gates']['determinism_failures']} |",
            "",
            f"Median local forwarded-byte reduction: `{report['median_forwarded_byte_reduction_ratio']}`.",
            "This is representation evidence from a local upstream only; it is not provider-token, cache, cost, or quality evidence.",
            "",
            "The default (`TRACEPRESS_ACTIVE_COMPRESSION=off`) remains byte-exact. `json.tabular` is not sent upstream because its TPJ2 representation has no provider decoding contract.",
            "",
        ]
    )


def main() -> int:
    try:
        report = run()
    except (BenchmarkError, subprocess.CalledProcessError) as error:
        print(f"active pilot failed: {error}", file=sys.stderr)
        return 1
    args = parse_args()
    args.output_json.parent.mkdir(parents=True, exist_ok=True)
    args.output_md.parent.mkdir(parents=True, exist_ok=True)
    args.output_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.output_md.write_text(markdown(report), encoding="utf-8")
    print(markdown(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
