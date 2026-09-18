#![cfg(unix)]
//! Subprocess contracts for session-scoped source reducer policy enforcement.

use std::{error::Error, fs, os::unix::fs::PermissionsExt as _, process::Command};

const TRACEPRESS: &str = env!("CARGO_BIN_EXE_tracepress");

#[test]
fn rejected_active_policy_forwards_exact_bytes_and_records_the_decision()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let fake_cargo = root.path().join("fake-cargo");
    fs::write(
        &fake_cargo,
        "#!/bin/sh\nprintf 'RAW_STDOUT\\n'\nprintf 'RAW_STDERR\\n' >&2\n",
    )?;
    fs::set_permissions(&fake_cargo, fs::Permissions::from_mode(0o700))?;

    let output = Command::new(TRACEPRESS)
        .args(["tool", "cargo", "check"])
        .env("TRACEPRESS_HOME", root.path().join("state"))
        .env("TRACEPRESS_REAL_CARGO", &fake_cargo)
        .env("TRACEPRESS_SOURCE_REDUCER", "cargo_check_v1_active")
        .output()?;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"RAW_STDOUT\n");
    assert_eq!(output.stderr, b"RAW_STDERR\n");

    let records = fs::read_to_string(root.path().join("state/source-executions.jsonl"))?;
    let record: serde_json::Value = serde_json::from_str(records.trim())?;
    assert_eq!(
        record.get("reducer_id").and_then(serde_json::Value::as_str),
        Some("cargo_check_v1_active")
    );
    assert_eq!(
        record
            .get("policy_decision")
            .and_then(serde_json::Value::as_str),
        Some("rejected")
    );
    assert_eq!(
        record
            .get("fail_open_reason")
            .and_then(serde_json::Value::as_str),
        Some("policy_rejected")
    );
    assert_eq!(
        record.get("active").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        record
            .get("forwarding_mutation")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        record
            .get("emitted_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(22)
    );
    Ok(())
}
