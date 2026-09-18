#!/usr/bin/env python3
"""Run the bounded Phase 5.0 Control versus passthrough A/A cohort.

Only aggregate allowlisted measurements cross the report boundary. Prompts, commands, tool
output, Codex output, paths, provider bodies, and source-execution rows remain local/transient.
"""
from __future__ import annotations
import argparse, json, os, shutil, sqlite3, subprocess, tempfile, time
from pathlib import Path
from typing import Any

EXPERIMENT = "source-passthrough-aa-001"
SHA = "3fce3b5bb0236da2df6d99672afb8a719642eca7"
URL = "https://github.com/BurntSushi/ripgrep"
MODEL = "gpt-5.6-luna"

def scalar(db: Path, query: str) -> int:
    with sqlite3.connect(f"file:{db}?mode=ro", uri=True) as con:
        return int(con.execute(query).fetchone()[0] or 0)

def agent_error_class(stderr: str) -> str | None:
    text = stderr.lower()
    if "sandbox" in text or "permission denied" in text: return "sandbox_or_permission"
    if "not found" in text or "no such file" in text: return "not_found"
    if "recursion" in text: return "recursion"
    return "other" if text else None

def reported_command_outcome(stdout: str, passed_sentinel: str, failed_sentinel: str) -> str:
    messages = []
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item") if event.get("type") == "item.completed" else None
        if isinstance(item, dict) and item.get("type") == "agent_message" and isinstance(item.get("text"), str):
            messages.append(item["text"])
    if not messages:
        return "unknown"
    final = messages[-1].strip().lower()
    if final == failed_sentinel.lower():
        return "failed"
    if final == passed_sentinel.lower():
        return "passed"
    return "unknown"

def aggregate(db: Path, state: Path) -> dict[str, Any]:
    with sqlite3.connect(f"file:{db}?mode=ro", uri=True) as con:
        usage = con.execute("SELECT SUM(input_total),SUM(input_cached),SUM(input_uncached),SUM(output_total),SUM(reasoning) FROM provider_usage").fetchone()
        requests = int(con.execute("SELECT COUNT(*) FROM provider_requests").fetchone()[0])
        errors = int(con.execute("SELECT COUNT(*) FROM provider_attempts WHERE status <> 'completed' OR error_code IS NOT NULL OR transport_error IS NOT NULL").fetchone()[0])
        provider_sessions = {row[0] for row in con.execute("SELECT DISTINCT o.session_id FROM provider_requests AS p JOIN operations AS o ON o.operation_id = p.operation_id")}
    source = state / "source-executions.jsonl"
    source_receipts = [json.loads(line) for line in source.open(encoding="utf-8")] if source.exists() else []
    source_rows = len(source_receipts)
    hooks = state / "hook-events.jsonl"
    hook_receipts = [json.loads(line) for line in hooks.open(encoding="utf-8")] if hooks.exists() else []
    hook_rows = len(hook_receipts)
    hook_rewrites = sum(receipt.get("rewritten") is True for receipt in hook_receipts)
    recoveries = state / "source-recovery-events.jsonl"
    recovery_receipts = [json.loads(line) for line in recoveries.open(encoding="utf-8")] if recoveries.exists() else []
    source_status_counts = {status: sum(receipt.get("exit_status_class") == status for receipt in source_receipts) for status in ("success", "nonzero", "signal")}
    source_status_counts["unknown"] = source_rows - sum(source_status_counts.values())
    return {"provider_requests": requests, "provider_errors": errors, "provider_usage": {"input_total": usage[0], "input_cached": usage[1], "input_uncached": usage[2], "output": usage[3], "reasoning": usage[4]}, "source_executions": source_rows, "source_exit_status_counts": source_status_counts, "source_successful_executions": source_status_counts["success"], "source_ids_present": sum(bool(receipt.get("source_execution_id")) for receipt in source_receipts), "source_sessions_linked": sum(receipt.get("session_id") in provider_sessions for receipt in source_receipts), "source_stdout_bytes": sum(receipt["raw_stdout_bytes"] for receipt in source_receipts), "source_stderr_bytes": sum(receipt["raw_stderr_bytes"] for receipt in source_receipts), "source_emitted_bytes": sum(receipt["emitted_bytes"] for receipt in source_receipts), "shadow_evaluations": sum(receipt.get("shadow") is True for receipt in source_receipts), "active_evaluations": sum(receipt.get("active") is True for receipt in source_receipts), "forwarding_mutations": sum(receipt.get("forwarding_mutation") is True for receipt in source_receipts), "fail_open_executions": sum(bool(receipt.get("fail_open_reason")) for receipt in source_receipts), "candidate_bytes": sum(receipt.get("candidate_bytes") or 0 for receipt in source_receipts), "estimated_raw_tokens": sum(receipt.get("estimated_raw_tokens") or 0 for receipt in source_receipts), "estimated_candidate_tokens": sum(receipt.get("estimated_candidate_tokens") or 0 for receipt in source_receipts), "never_worse_accepted": sum(receipt.get("never_worse_accepted") is True for receipt in source_receipts), "omitted_passing_tests": sum(receipt.get("omitted_passing_tests") or 0 for receipt in source_receipts), "omitted_progress_lines": sum(receipt.get("omitted_progress_lines") or 0 for receipt in source_receipts), "grouped_match_lines": sum(receipt.get("grouped_match_lines") or 0 for receipt in source_receipts), "recovery_hint_bytes": sum(receipt.get("recovery_hint_bytes") or 0 for receipt in source_receipts), "reducer_duration_us": sum(receipt.get("reducer_duration_us") or 0 for receipt in source_receipts), "recovery_requests": len(recovery_receipts), "recovered_bytes": sum((receipt.get("recovered_stdout_bytes") or 0) + (receipt.get("recovered_stderr_bytes") or 0) for receipt in recovery_receipts), "tool_calls": source_rows + len(recovery_receipts), "command_retries": max(0, source_rows - 1), "hook_events": hook_rows, "hook_rewrites": hook_rewrites}

def checkout(root: Path) -> Path:
    repo = root / "ripgrep"
    if not (repo / ".git").exists():
        root.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git","clone","--filter=blob:none",URL,str(repo)],check=True,capture_output=True,text=True,timeout=90)
    subprocess.run(["git","-C",str(repo),"checkout","--detach",SHA],check=True,capture_output=True,text=True,timeout=30)
    return repo

def isolated_worktree(repo: Path) -> Path:
    """Create one clean pinned worktree so Cargo build output cannot cross arms."""
    path = Path(tempfile.mkdtemp(prefix="tp5-aa-worktree-", dir="/tmp"))
    path.rmdir()
    subprocess.run(
        ["git", "-C", str(repo), "worktree", "add", "--detach", str(path), SHA],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    )
    return path

def remove_worktree(repo: Path, path: Path) -> None:
    subprocess.run(
        ["git", "-C", str(repo), "worktree", "remove", "--force", str(path)],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    shutil.rmtree(path, ignore_errors=True)

def arm(cli: Path, daemon: Path, repo: Path, name: str, timeout: int, experiment: str, check_scenario: str, rg_scenario: str) -> dict[str, Any]:
    workspace = isolated_worktree(repo)
    state = Path(tempfile.mkdtemp(prefix="tp5-aa-", dir="/tmp")); log = None; process = None
    started = time.monotonic()
    try:
        env = os.environ.copy(); env.update({"TRACEPRESS_HOME":str(state),"TRACEPRESS_CONTEXT_ANALYSIS":"shadow","TRACEPRESS_SHADOW_COMPRESSION":"off","TRACEPRESS_ACTIVE_COMPRESSION":"off","TRACEPRESS_MEASUREMENT_RUN_ID":experiment,"TRACEPRESS_TOOL_BIN":str(cli),"RUST_TEST_THREADS":"1","CARGO_TARGET_DIR":str(state / "cargo-target")})
        if name in {"Passthrough", "Identity", "Path"}:
            env["TRACEPRESS_SOURCE_HOOK"] = "path" if name == "Path" else "codex"
        if name == "Identity": env["TRACEPRESS_HOOK_REWRITE_MODE"] = "identity"
        if name == "ExplicitShadow": env["TRACEPRESS_SOURCE_REDUCER"] = "cargo_test_v1_shadow"
        if name == "ExplicitCheckShadow": env["TRACEPRESS_SOURCE_REDUCER"] = "cargo_check_v1_shadow"
        if name == "ExplicitCheckActive": env["TRACEPRESS_SOURCE_REDUCER"] = "cargo_check_v1_active"
        if name == "ExplicitCheckV2Shadow": env["TRACEPRESS_SOURCE_REDUCER"] = "cargo_check_v2_shadow"
        if name == "ExplicitCheckV2Active": env["TRACEPRESS_SOURCE_REDUCER"] = "cargo_check_v2_active"
        if name == "ExplicitClippyShadow": env["TRACEPRESS_SOURCE_REDUCER"] = "cargo_clippy_v1_shadow"
        if name == "ExplicitRgShadow": env["TRACEPRESS_SOURCE_REDUCER"] = "rg_v1_shadow"
        if name == "ExplicitRgActive": env["TRACEPRESS_SOURCE_REDUCER"] = "rg_v1_active"
        if name == "ExplicitActive": env["TRACEPRESS_SOURCE_REDUCER"] = "cargo_test_v1_active"
        subprocess.run([str(cli),"init"],cwd=workspace,env=env,check=True,capture_output=True,text=True,timeout=30)
        daemon_env = dict(env); daemon_env.update({"TRACEPRESS_DATABASE":str(state/"tracepress.sqlite3"),"TRACEPRESS_CONTROL_SOCKET":str(state/"tracepress.sock"),"TRACEPRESS_CONTROL_CREDENTIAL":str(state/"control.cred"),"TRACEPRESS_DAEMON_READY":str(state/"daemon.ready")})
        log = (state/"daemon.log").open("w",encoding="utf-8"); process = subprocess.Popen([str(daemon)],cwd=workspace,env=daemon_env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True,text=True)
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            if process.poll() is not None: raise RuntimeError("daemon exited before readiness")
            if (state/"daemon.ready").exists(): break
            time.sleep(.1)
        else: raise RuntimeError("daemon did not become ready")
        if name in {"ExplicitRgControl", "ExplicitRgShadow", "ExplicitRgActiveControl", "ExplicitRgActive"}:
            rg_args = "rg -n struct crates" if rg_scenario == "parseable" else "rg -n fn crates"
            recovery = " If and only if the output is insufficient, you may run the exact Tracepress recovery command printed in the output or rerun the same wrapper command." if name in {"ExplicitRgActiveControl", "ExplicitRgActive"} else ""
            prompt=f"Your first Bash command must be exactly `{cli} tool {rg_args}`.{recovery} Do not run any other Bash command, inspect files, or modify source. End with exactly SEARCH_MATCHED if the command finds matches or SEARCH_NO_MATCH if it finds none, and no other text."
            passed_sentinel, failed_sentinel = "SEARCH_MATCHED", "SEARCH_NO_MATCH"
        elif name in {"ExplicitClippyControl", "ExplicitClippyShadow"}:
            prompt=f"Your first and only Bash command must be exactly `{cli} tool cargo clippy`. Do not run any other Bash command, inspect files, or modify source. End with exactly CLIPPY_PASSED if the command succeeds or CLIPPY_FAILED if it fails, and no other text."
            passed_sentinel, failed_sentinel = "CLIPPY_PASSED", "CLIPPY_FAILED"
        elif name in {"ExplicitCheckControl", "ExplicitCheckShadow", "ExplicitCheckActiveControl", "ExplicitCheckActive", "ExplicitCheckV2Control", "ExplicitCheckV2Shadow", "ExplicitCheckV2Active"}:
            check_args = "cargo check" if check_scenario == "success" else "cargo check --package tracepress_missing_package"
            recovery = " If and only if the output is insufficient, you may run the exact Tracepress recovery command printed in the output or rerun the same wrapper command." if name in {"ExplicitCheckActiveControl", "ExplicitCheckActive", "ExplicitCheckV2Control", "ExplicitCheckV2Active"} else ""
            prompt=f"Your first Bash command must be exactly `{cli} tool {check_args}`.{recovery} Do not run any other Bash command, inspect files, or modify source. End with exactly CHECK_PASSED if the command succeeds or CHECK_FAILED if it fails, and no other text."
            passed_sentinel, failed_sentinel = "CHECK_PASSED", "CHECK_FAILED"
        elif name in {"ExplicitActiveControl", "ExplicitActive"}:
            prompt=f"Your first Bash command must be exactly `{cli} tool cargo test`. Use its output to determine whether tests passed. If and only if the output is insufficient, you may run the exact Tracepress recovery command printed in the output or rerun the same wrapper command. Do not inspect files, modify source, or run any other Bash command. End with exactly TESTS_PASSED or TESTS_FAILED and no other text."
            passed_sentinel, failed_sentinel = "TESTS_PASSED", "TESTS_FAILED"
        elif name.startswith("Explicit"):
            prompt=f"Your first and only Bash command must be exactly `{cli} tool cargo test`. Do not run any other Bash command, inspect files, or modify source. After it completes, report only whether tests passed."
            passed_sentinel, failed_sentinel = "TESTS_PASSED", "TESTS_FAILED"
        else:
            prompt="Your first and only Bash command must be exactly `cargo test`. Do not run any other Bash command, inspect files, or modify source. After it completes, report only whether tests passed."
            passed_sentinel, failed_sentinel = "TESTS_PASSED", "TESTS_FAILED"
        result=subprocess.run([str(cli),"run","codex","exec","--json","-m",MODEL,"-s","workspace-write","--skip-git-repo-check",prompt],cwd=workspace,env=env,capture_output=True,text=True,timeout=timeout)
        summary=aggregate(state/"tracepress.sqlite3",state); summary.update({"arm":name,"agent_exit_status_class":"success" if result.returncode==0 else "nonzero","agent_error_class":None if result.returncode==0 else agent_error_class(result.stderr),"duration_ms":round((time.monotonic()-started)*1000),"timed_out":False})
        actual_outcome = "passed" if summary["source_successful_executions"] == summary["source_executions"] else "failed"
        reported_outcome = reported_command_outcome(result.stdout, passed_sentinel, failed_sentinel)
        summary.update({"actual_outcome":actual_outcome,"reported_outcome":reported_outcome,"task_success":result.returncode == 0 and summary["source_executions"] >= 1 and summary["provider_errors"] == 0 and reported_outcome == actual_outcome})
        return summary
    except subprocess.TimeoutExpired:
        return {"arm":name,"agent_exit_status_class":"timeout","duration_ms":round((time.monotonic()-started)*1000),"timed_out":True}
    finally:
        if process is not None: process.terminate(); process.wait(timeout=10)
        if log is not None: log.close()
        shutil.rmtree(state,ignore_errors=True)
        remove_worktree(repo, workspace)

def main() -> int:
    p=argparse.ArgumentParser(description=__doc__); p.add_argument("--repo-root",type=Path,default=Path(__file__).resolve().parents[1]); p.add_argument("--workload-root",type=Path,default=Path("/tmp/tracepress-source-passthrough-aa-001")); p.add_argument("--pairs",type=int,default=1,choices=range(1,11)); p.add_argument("--timeout",type=int,default=180); p.add_argument("--hook-mode",choices=("passthrough","identity","path","explicit-aa","explicit-shadow","explicit-active","explicit-check-shadow","explicit-check-active","explicit-check-shadow-v2","explicit-check-active-v2","explicit-clippy-shadow","explicit-rg-shadow","explicit-rg-active"),default="passthrough"); p.add_argument("--check-scenario",choices=("success","diagnostic-failure"),default="success"); p.add_argument("--rg-scenario",choices=("parseable","ambiguous"),default="parseable"); p.add_argument("--output-json",type=Path,required=True); p.add_argument("--output-md",type=Path,required=True); a=p.parse_args()
    cli=a.repo_root/"target/debug/tracepress"; daemon=a.repo_root/"target/debug/tracepressd"
    if not cli.exists() or not daemon.exists(): raise RuntimeError("build target/debug/tracepress and target/debug/tracepressd first")
    treatment = {"passthrough": "Passthrough", "identity": "Identity", "path": "Path", "explicit-aa": "ExplicitB", "explicit-shadow": "ExplicitShadow", "explicit-active": "ExplicitActive", "explicit-check-shadow": "ExplicitCheckShadow", "explicit-check-active": "ExplicitCheckActive", "explicit-check-shadow-v2": "ExplicitCheckV2Shadow", "explicit-check-active-v2": "ExplicitCheckV2Active", "explicit-clippy-shadow": "ExplicitClippyShadow", "explicit-rg-shadow": "ExplicitRgShadow", "explicit-rg-active": "ExplicitRgActive"}[a.hook_mode]
    control = "ExplicitA" if a.hook_mode == "explicit-aa" else ("ExplicitActiveControl" if a.hook_mode == "explicit-active" else ("ExplicitControl" if a.hook_mode == "explicit-shadow" else ("ExplicitCheckControl" if a.hook_mode == "explicit-check-shadow" else ("ExplicitCheckActiveControl" if a.hook_mode == "explicit-check-active" else ("ExplicitCheckV2Control" if a.hook_mode in {"explicit-check-shadow-v2","explicit-check-active-v2"} else ("ExplicitClippyControl" if a.hook_mode == "explicit-clippy-shadow" else ("ExplicitRgControl" if a.hook_mode == "explicit-rg-shadow" else ("ExplicitRgActiveControl" if a.hook_mode == "explicit-rg-active" else "Control"))))))))
    experiment = "source-active-pilot-001" if a.hook_mode == "explicit-active" else ("source-cargo-check-shadow-001" if a.hook_mode == "explicit-check-shadow" else (f"source-cargo-check-active-{a.check_scenario}-001" if a.hook_mode == "explicit-check-active" else ("source-cargo-check-v2-shadow-001" if a.hook_mode == "explicit-check-shadow-v2" else (f"source-cargo-check-v2-active-{a.check_scenario}-001" if a.hook_mode == "explicit-check-active-v2" else ("source-cargo-clippy-shadow-001" if a.hook_mode == "explicit-clippy-shadow" else ("source-rg-shadow-001" if a.hook_mode == "explicit-rg-shadow" else (f"source-rg-active-{a.rg_scenario}-001" if a.hook_mode == "explicit-rg-active" else EXPERIMENT)))))))
    repo=checkout(a.workload_root); rows=[]
    for pair in range(a.pairs):
        order = [control, treatment] if pair % 2 == 0 else [treatment, control]
        rows.extend(arm(cli,daemon,repo,name,a.timeout,experiment,a.check_scenario,a.rg_scenario) for name in order)
    if a.hook_mode.startswith("explicit"):
        source_complete = all(row.get("hook_rewrites", 0) == 0 and row.get("source_executions", 0) >= 1 for row in rows)
    else:
        source_complete = all(row.get("hook_rewrites", 0) == 1 and (a.hook_mode == "identity" or row.get("source_executions", 0) == 1) for row in rows if row["arm"] == treatment)
    phase = "5.6" if a.hook_mode == "explicit-rg-active" else ("5.5" if a.hook_mode == "explicit-rg-shadow" else ("5.4" if a.hook_mode == "explicit-clippy-shadow" else ("5.3" if a.hook_mode in {"explicit-check-active","explicit-check-shadow-v2","explicit-check-active-v2"} else ("5.2" if a.hook_mode == "explicit-check-shadow" else ("5.1" if a.hook_mode == "explicit-active" else "5.0")))))
    scenario = a.rg_scenario if a.hook_mode == "explicit-rg-active" else (a.check_scenario if a.hook_mode in {"explicit-check-active","explicit-check-shadow-v2","explicit-check-active-v2"} else None)
    report={"experiment_id":experiment,"phase":phase,"status":"completed","pairs":a.pairs,"reducer":a.hook_mode,"scenario":scenario,"forwarding_mutation":any(row.get("forwarding_mutations",0)>0 for row in rows),"repository_pin":{"repository":"BurntSushi/ripgrep","commit_sha":SHA},"workload_isolation":{"per_arm_clean_git_worktree":True,"per_arm_cargo_target_dir":True,"arm_order":"alternating","rust_test_threads":1},"rows":rows,"infrastructure_gate":{"source_execution_per_passthrough_session":source_complete,"decision":"instrumentation_valid" if source_complete else "instrumentation_invalid_missing_source_execution"},"privacy":{"commands_persisted":False,"paths_persisted":False,"raw_content_persisted":False}}
    a.output_json.parent.mkdir(parents=True,exist_ok=True); a.output_json.write_text(json.dumps(report,indent=2,sort_keys=True)+"\n",encoding="utf-8")
    a.output_md.write_text("# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001\n\nStatus: **completed**. Passthrough only; no reducer was enabled.\n",encoding="utf-8")
    return 0
if __name__ == "__main__": raise SystemExit(main())
