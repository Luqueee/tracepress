//! Subprocess contract tests for the metadata-only public tool shadow evaluator.

use std::{
    error::Error,
    fmt::Write as FmtWrite,
    io::{self, Write as IoWrite},
    process::{Command, Output, Stdio},
};

const TOOL_SHADOW: &str = env!("CARGO_BIN_EXE_tracepress-tool-shadow");

fn run_shadow(family: &str, input: &[u8]) -> Result<Output, Box<dyn Error>> {
    let mut child = Command::new(TOOL_SHADOW)
        .arg(family)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("tool shadow stdin was not piped"))?;
    if let Err(error) = stdin.write_all(input)
        && error.kind() != io::ErrorKind::BrokenPipe
    {
        return Err(error.into());
    }
    drop(stdin);
    Ok(child.wait_with_output()?)
}

#[test]
fn search_shadow_emits_only_bounded_metadata() -> Result<(), Box<dyn Error>> {
    const CANARY: &str = "PRIVATE_TOOL_RESULT_CANARY";
    let mut input = String::new();
    for line in 1..=12 {
        writeln!(&mut input, "src/private.rs:{line}:{CANARY}_{line}")?;
    }

    let output = run_shadow("search", input.as_bytes())?;
    assert!(output.status.success(), "{output:?}");
    assert!(
        !output
            .stdout
            .windows(CANARY.len())
            .any(|item| item == CANARY.as_bytes())
    );

    let body: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        body.get("family").and_then(serde_json::Value::as_str),
        Some("search")
    );
    assert_eq!(
        body.get("input_bytes").and_then(serde_json::Value::as_u64),
        u64::try_from(input.len()).ok()
    );
    assert_eq!(
        body.get("recovery_verified")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        body.get("deterministic")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        body.get("canonical_equal")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    Ok(())
}

#[test]
fn shadow_rejects_unknown_families() -> Result<(), Box<dyn Error>> {
    let output = run_shadow("shell", b"src/lib.rs:1:value\n")?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)?.contains("only the bounded search shadow is enabled")
    );
    Ok(())
}

#[test]
fn shadow_rejects_input_above_its_public_workload_limit() -> Result<(), Box<dyn Error>> {
    let oversized = vec![b'x'; 1_048_577];
    let output = run_shadow("search", &oversized)?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)?
            .contains("input exceeds bounded public-workload shadow limit")
    );
    Ok(())
}
