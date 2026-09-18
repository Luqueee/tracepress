use crate::{admission::decide, model::RewriteDecision};

/// Builds a fail-open Codex `PreToolUse` rewrite response without granting permission.
///
/// Malformed, non-Bash, unsupported, or syntactically unsafe calls return `None`, which means
/// the hook writes nothing and Codex executes the original command.
#[must_use]
pub fn codex_pre_tool_use_rewrite(input: &[u8]) -> Option<Vec<u8>> {
    let payload: serde_json::Value = serde_json::from_slice(input).ok()?;
    if payload.get("hook_event_name")?.as_str()? != "PreToolUse"
        || payload.get("tool_name")?.as_str()? != "Bash"
    {
        return None;
    }
    let command = payload.pointer("/tool_input/command")?.as_str()?;
    let RewriteDecision::Passthrough { .. } = decide(command) else {
        return None;
    };
    let executable = std::env::var("TRACEPRESS_TOOL_BIN")
        .ok()
        .filter(|value| !value.is_empty() && !value.contains([' ', '\'', '"', '$', '`', '\\']))
        .unwrap_or_else(|| "tracepress".to_owned());
    let session = payload
        .get("session_id")
        .or_else(|| payload.get("turn_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| {
            value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    let session_prefix = session.map_or_else(String::new, |value| {
        format!("TRACEPRESS_SOURCE_SESSION_ID={value} ")
    });
    serde_json::to_vec(&serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": { "command": format!("{session_prefix}{executable} tool {command}") }
        }
    }))
    .ok()
}

/// Builds an admitted Codex rewrite that preserves the original command text.
#[must_use]
pub fn codex_pre_tool_use_identity_rewrite(input: &[u8]) -> Option<Vec<u8>> {
    let payload: serde_json::Value = serde_json::from_slice(input).ok()?;
    if payload.get("hook_event_name")?.as_str()? != "PreToolUse"
        || payload.get("tool_name")?.as_str()? != "Bash"
    {
        return None;
    }
    let command = payload.pointer("/tool_input/command")?.as_str()?;
    let RewriteDecision::Passthrough { .. } = decide(command) else {
        return None;
    };
    serde_json::to_vec(&serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": { "command": command }
        }
    }))
    .ok()
}

#[cfg(test)]
#[allow(clippy::panic, reason = "test fixtures use explicit failure branches")]
mod tests {
    use super::*;

    fn rewritten_command(input: &[u8]) -> Option<String> {
        let output = codex_pre_tool_use_rewrite(input)?;
        serde_json::from_slice::<serde_json::Value>(&output)
            .ok()?
            .pointer("/hookSpecificOutput/updatedInput/command")?
            .as_str()
            .map(str::to_owned)
    }

    #[test]
    fn rewrite_is_narrow_and_does_not_grant_extra_behavior() {
        let cargo = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test -q"}}"#;
        let Some(output) = codex_pre_tool_use_rewrite(cargo) else {
            panic!("admitted command must rewrite");
        };
        let Ok(value): Result<serde_json::Value, _> = serde_json::from_slice(&output) else {
            panic!("rewrite response must be valid JSON");
        };
        assert_eq!(
            value
                .pointer("/hookSpecificOutput/permissionDecision")
                .and_then(serde_json::Value::as_str),
            Some("allow")
        );
        assert!(
            value
                .pointer("/hookSpecificOutput/updatedInput/command")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|command| command.ends_with(" tool cargo test -q"))
        );
        let git = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git status"}}"#;
        assert!(rewritten_command(git).is_some_and(|value| value.ends_with(" tool git status")));
        assert!(codex_pre_tool_use_rewrite(br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test | tail"}}"#).is_none());
        assert!(codex_pre_tool_use_rewrite(br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git status --short"}}"#).is_none());
    }

    #[test]
    fn identity_rewrite_preserves_the_admitted_command() {
        let input = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test -q"}}"#;
        let Some(output) = codex_pre_tool_use_identity_rewrite(input) else {
            panic!("admitted command must produce an identity rewrite");
        };
        let Ok(value): Result<serde_json::Value, _> = serde_json::from_slice(&output) else {
            panic!("identity response must be valid JSON");
        };
        assert_eq!(
            value
                .pointer("/hookSpecificOutput/updatedInput/command")
                .and_then(serde_json::Value::as_str),
            Some("cargo test -q")
        );
    }
}
