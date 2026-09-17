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

def aggregate(db: Path, state: Path) -> dict[str, Any]:
    with sqlite3.connect(f"file:{db}?mode=ro", uri=True) as con:
        usage = con.execute("SELECT SUM(input_total),SUM(input_cached),SUM(input_uncached),SUM(output_total),SUM(reasoning) FROM provider_usage").fetchone()
        requests = int(con.execute("SELECT COUNT(*) FROM provider_requests").fetchone()[0])
        errors = int(con.execute("SELECT COUNT(*) FROM provider_attempts WHERE status <> 'completed' OR error_code IS NOT NULL OR transport_error IS NOT NULL").fetchone()[0])
    source = state / "source-executions.jsonl"
    source_rows = sum(1 for _ in source.open(encoding="utf-8")) if source.exists() else 0
    hooks = state / "hook-events.jsonl"
    hook_receipts = [json.loads(line) for line in hooks.open(encoding="utf-8")] if hooks.exists() else []
    hook_rows = len(hook_receipts)
    hook_rewrites = sum(receipt.get("rewritten") is True for receipt in hook_receipts)
    return {"provider_requests": requests, "provider_errors": errors, "provider_usage": {"input_total": usage[0], "input_cached": usage[1], "input_uncached": usage[2], "output": usage[3], "reasoning": usage[4]}, "source_executions": source_rows, "hook_events": hook_rows, "hook_rewrites": hook_rewrites}

def checkout(root: Path) -> Path:
    repo = root / "ripgrep"
    if not (repo / ".git").exists():
        root.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git","clone","--filter=blob:none",URL,str(repo)],check=True,capture_output=True,text=True,timeout=90)
    subprocess.run(["git","-C",str(repo),"checkout","--detach",SHA],check=True,capture_output=True,text=True,timeout=30)
    return repo

def arm(cli: Path, daemon: Path, repo: Path, name: str, timeout: int) -> dict[str, Any]:
    state = Path(tempfile.mkdtemp(prefix="tp5-aa-", dir="/tmp")); log = None; process = None
    started = time.monotonic()
    try:
        env = os.environ.copy(); env.update({"TRACEPRESS_HOME":str(state),"TRACEPRESS_CONTEXT_ANALYSIS":"shadow","TRACEPRESS_SHADOW_COMPRESSION":"off","TRACEPRESS_ACTIVE_COMPRESSION":"off","TRACEPRESS_MEASUREMENT_RUN_ID":EXPERIMENT})
        if name == "Passthrough": env["TRACEPRESS_SOURCE_HOOK"] = "codex"
        subprocess.run([str(cli),"init"],cwd=repo,env=env,check=True,capture_output=True,text=True,timeout=30)
        daemon_env = dict(env); daemon_env.update({"TRACEPRESS_DATABASE":str(state/"tracepress.sqlite3"),"TRACEPRESS_CONTROL_SOCKET":str(state/"tracepress.sock"),"TRACEPRESS_CONTROL_CREDENTIAL":str(state/"control.cred"),"TRACEPRESS_DAEMON_READY":str(state/"daemon.ready")})
        log = (state/"daemon.log").open("w",encoding="utf-8"); process = subprocess.Popen([str(daemon)],cwd=repo,env=daemon_env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True,text=True)
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            if process.poll() is not None: raise RuntimeError("daemon exited before readiness")
            if (state/"daemon.ready").exists(): break
            time.sleep(.1)
        else: raise RuntimeError("daemon did not become ready")
        prompt="Your first and only Bash command must be exactly `cargo test`. Do not run any other Bash command, inspect files, or modify source. After it completes, report only whether tests passed."
        result=subprocess.run([str(cli),"run","codex","exec","-m",MODEL,"-s","workspace-write","--skip-git-repo-check",prompt],cwd=repo,env=env,capture_output=True,text=True,timeout=timeout)
        summary=aggregate(state/"tracepress.sqlite3",state); summary.update({"arm":name,"agent_exit_status_class":"success" if result.returncode==0 else "nonzero","duration_ms":round((time.monotonic()-started)*1000),"timed_out":False})
        return summary
    except subprocess.TimeoutExpired:
        return {"arm":name,"agent_exit_status_class":"timeout","duration_ms":round((time.monotonic()-started)*1000),"timed_out":True}
    finally:
        if process is not None: process.terminate(); process.wait(timeout=10)
        if log is not None: log.close()
        shutil.rmtree(state,ignore_errors=True)

def main() -> int:
    p=argparse.ArgumentParser(description=__doc__); p.add_argument("--repo-root",type=Path,default=Path(__file__).resolve().parents[1]); p.add_argument("--workload-root",type=Path,default=Path("/tmp/tracepress-source-passthrough-aa-001")); p.add_argument("--pairs",type=int,default=1,choices=range(1,11)); p.add_argument("--timeout",type=int,default=180); p.add_argument("--output-json",type=Path,required=True); p.add_argument("--output-md",type=Path,required=True); a=p.parse_args()
    cli=a.repo_root/"target/debug/tracepress"; daemon=a.repo_root/"target/debug/tracepressd"
    if not cli.exists() or not daemon.exists(): raise RuntimeError("build target/debug/tracepress and target/debug/tracepressd first")
    repo=checkout(a.workload_root); rows=[]
    for _ in range(a.pairs):
        rows.extend([arm(cli,daemon,repo,"Control",a.timeout),arm(cli,daemon,repo,"Passthrough",a.timeout)])
    source_complete = all(row.get("source_executions", 0) == 1 and row.get("hook_rewrites", 0) == 1 for row in rows if row["arm"] == "Passthrough")
    report={"experiment_id":EXPERIMENT,"phase":"5.0","status":"completed","pairs":a.pairs,"reducer":"passthrough","forwarding_mutation":False,"repository_pin":{"repository":"BurntSushi/ripgrep","commit_sha":SHA},"rows":rows,"infrastructure_gate":{"source_execution_per_passthrough_session":source_complete,"decision":"aa_valid" if source_complete else "aa_invalid_missing_source_execution"},"privacy":{"commands_persisted":False,"paths_persisted":False,"raw_content_persisted":False}}
    a.output_json.parent.mkdir(parents=True,exist_ok=True); a.output_json.write_text(json.dumps(report,indent=2,sort_keys=True)+"\n",encoding="utf-8")
    a.output_md.write_text("# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001\n\nStatus: **completed**. Passthrough only; no reducer was enabled.\n",encoding="utf-8")
    return 0
if __name__ == "__main__": raise SystemExit(main())
