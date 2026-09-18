//! Codex observation handshake and fail-open hook handling.

use super::*;

#[allow(
    clippy::too_many_lines,
    reason = "the handshake transcript stays linear so protocol ordering remains auditable"
)]
pub(super) fn codex_observe(command_smoke: bool) -> Result<(), String> {
    let mut child = std::process::Command::new("codex")
        .args(["app-server", "--stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start Codex app-server: {error}"))?;
    let request = serde_json::json!({
        "id": 1,
        "method": "initialize",
        "params": {"clientInfo": {"name": "tracepress-observer", "version": env!("CARGO_PKG_VERSION")}, "capabilities": {"experimentalApi": true}}
    });
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| "app-server stdin unavailable".to_owned())?;
    serde_json::to_writer(&mut input, &request).map_err(|error| error.to_string())?;
    input.write_all(b"\n").map_err(|error| error.to_string())?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| "app-server stdout unavailable".to_owned())?;
    let mut lines = std::io::BufReader::new(output).lines();
    let response = lines
        .next()
        .ok_or_else(|| "app-server closed before initialize".to_owned())?
        .map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&response).map_err(|error| error.to_string())?;
    if !command_smoke {
        let _killed = child.kill();
        println!(
            "{}",
            serde_json::json!({"event":"initialize","accepted":value.get("result").is_some(),"output_persisted":false,"thread_started":false,"turn_started":false})
        );
        return Ok(());
    }
    let thread = serde_json::json!({"id":2,"method":"thread/start","params":{"cwd":std::env::current_dir().map_err(|error| error.to_string())?,"approvalPolicy":"never","sandbox":"read-only","ephemeral":true}});
    serde_json::to_writer(&mut input, &thread).map_err(|error| error.to_string())?;
    input.write_all(b"\n").map_err(|error| error.to_string())?;
    let mut event_counts = BTreeMap::<String, u64>::new();
    let thread_id = loop {
        let line = lines
            .next()
            .ok_or_else(|| "app-server closed before thread/start".to_owned())?
            .map_err(|error| error.to_string())?;
        let event: serde_json::Value =
            serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if let Some(method) = event.get("method").and_then(serde_json::Value::as_str) {
            let count = event_counts.entry(method.to_owned()).or_default();
            *count = count.saturating_add(1);
        }
        if event.get("id").and_then(serde_json::Value::as_u64) == Some(2) {
            break event
                .pointer("/result/thread/id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "thread/start returned no thread id".to_owned())?
                .to_owned();
        }
    };
    let turn = serde_json::json!({"id":3,"method":"turn/start","params":{"threadId":thread_id,"input":[{"type":"text","text":"Run exactly `printf TRACEPRESS_APPSERVER_OBSERVER` once, then reply only done."}]}});
    serde_json::to_writer(&mut input, &turn).map_err(|error| error.to_string())?;
    input.write_all(b"\n").map_err(|error| error.to_string())?;
    let mut output_delta_chars = 0_u64;
    let mut aggregated_output_chars = 0_u64;
    let mut item_types = BTreeMap::<String, u64>::new();
    let mut turn_started = false;
    loop {
        let line = lines
            .next()
            .ok_or_else(|| "app-server closed before turn completion".to_owned())?
            .map_err(|error| error.to_string())?;
        let event: serde_json::Value =
            serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if event.get("id").and_then(serde_json::Value::as_u64) == Some(3) {
            turn_started = event.get("result").is_some();
        }
        if let Some(method) = event.get("method").and_then(serde_json::Value::as_str) {
            let count = event_counts.entry(method.to_owned()).or_default();
            *count = count.saturating_add(1);
            if matches!(method, "item/started" | "item/completed")
                && let Some(item_type) = event
                    .pointer("/params/item/type")
                    .and_then(serde_json::Value::as_str)
            {
                let count = item_types.entry(item_type.to_owned()).or_default();
                *count = count.saturating_add(1);
                if item_type == "commandExecution" && method == "item/completed" {
                    aggregated_output_chars = aggregated_output_chars.saturating_add(
                        event
                            .pointer("/params/item/aggregatedOutput")
                            .and_then(serde_json::Value::as_str)
                            .map_or(0, |output| u64::try_from(output.len()).unwrap_or(u64::MAX)),
                    );
                }
            }
            if method == "item/commandExecution/outputDelta" {
                output_delta_chars = output_delta_chars.saturating_add(
                    event
                        .pointer("/params/delta")
                        .and_then(serde_json::Value::as_str)
                        .map_or(0, |delta| u64::try_from(delta.len()).unwrap_or(u64::MAX)),
                );
            }
            if method == "turn/completed" {
                break;
            }
        }
    }
    let _killed = child.kill();
    println!(
        "{}",
        serde_json::json!({"event":"command_smoke","accepted":value.get("result").is_some(),"output_persisted":false,"thread_started":true,"turn_started":turn_started,"event_counts":event_counts,"item_types":item_types,"command_output_delta_chars":output_delta_chars,"command_aggregated_output_chars":aggregated_output_chars})
    );
    Ok(())
}

pub(super) fn hook(agent: &str) -> Result<(), String> {
    if agent != "codex" {
        return Err("only the Codex hook adapter is supported".to_owned());
    }
    let mut input = Vec::new();
    let _read = std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|error| format!("cannot read hook input: {error}"))?;
    let mode = std::env::var("TRACEPRESS_HOOK_REWRITE_MODE").ok();
    let output = if mode.as_deref() == Some("observe") {
        None
    } else if mode.as_deref() == Some("identity") {
        codex_pre_tool_use_identity_rewrite(&input)
    } else {
        codex_pre_tool_use_rewrite(&input)
    };
    let _recorded = record_hook_event(&input, output.is_some());
    if let Some(output) = output {
        std::io::stdout()
            .write_all(&output)
            .map_err(|error| format!("cannot write hook output: {error}"))?;
    }
    Ok(())
}

/// Appends an allowlisted hook receipt; hook telemetry must never retain the command payload.
pub(super) fn record_hook_event(input: &[u8], rewritten: bool) -> Result<(), String> {
    let root = std::env::var_os("TRACEPRESS_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "missing Tracepress home".to_owned())?;
    let value: serde_json::Value =
        serde_json::from_slice(input).map_err(|error| error.to_string())?;
    let event = serde_json::json!({
        "hook_event": value.get("hook_event_name").and_then(serde_json::Value::as_str),
        "tool_name": value.get("tool_name").and_then(serde_json::Value::as_str),
        "rewritten": rewritten,
    });
    let mut options = std::fs::OpenOptions::new();
    let _options = options.create(true).append(true);
    #[cfg(unix)]
    let _mode = options.mode(0o600);
    let mut file = options
        .open(root.join("hook-events.jsonl"))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut file, &event).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())
}
