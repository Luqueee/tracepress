//! Source-side tool execution, reduction policy, and byte-faithful recovery.

use super::*;

pub(super) const SOURCE_RECOVERY_TTL: Duration = Duration::from_secs(60 * 60);
pub(super) const SOURCE_RECOVERY_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(super) struct SourceRecoveryMetadata {
    pub(super) version: u32,
    pub(super) source_execution_id: String,
    pub(super) created_at_unix: u64,
    pub(super) expires_at_unix: u64,
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SourceRecoveryWrite<'a> {
    pub(super) session_id: &'a str,
    pub(super) token: &'a str,
    pub(super) source_execution_id: SourceExecutionId,
    pub(super) stdout: &'a [u8],
    pub(super) stderr: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SourceEmissionRecord<'a> {
    candidate: Option<&'a CargoOutputCandidate>,
    reducer_id: &'static str,
    policy_decision: &'static str,
    emitted_bytes: u64,
    active: bool,
    recovery_available: bool,
    fail_open_reason: Option<&'static str>,
}

pub(super) fn source_session_id() -> Option<String> {
    std::env::var("TRACEPRESS_SOURCE_SESSION_ID")
        .ok()
        .or_else(|| std::env::var("TRACEPRESS_SESSION_ID").ok())
        .filter(|value| value.parse::<SessionId>().is_ok())
}

pub(super) fn source_recovery_token() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn valid_source_recovery_token(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn safe_source_recovery_executable(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
}

pub(super) fn unix_now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())
        .map(|duration| duration.as_secs())
}

pub(super) fn write_private_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    let _options = options.write(true).create_new(true);
    #[cfg(unix)]
    let _mode = options.mode(0o600);
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

pub(super) fn cleanup_expired_source_recoveries(session_root: &std::path::Path, now: u64) {
    let Ok(entries) = std::fs::read_dir(session_root) else {
        return;
    };
    for entry in entries.flatten() {
        let token = entry.file_name();
        let token = token.to_string_lossy();
        if !valid_source_recovery_token(&token) {
            continue;
        }
        let expired = std::fs::read(entry.path().join("metadata.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<SourceRecoveryMetadata>(&bytes).ok())
            .is_some_and(|metadata| metadata.expires_at_unix < now);
        if expired {
            let _removed = std::fs::remove_dir_all(entry.path());
        }
    }
}

pub(super) fn store_source_recovery(
    config: &Config,
    write: SourceRecoveryWrite<'_>,
) -> Result<(), String> {
    let raw_bytes =
        u64::try_from(write.stdout.len().saturating_add(write.stderr.len())).unwrap_or(u64::MAX);
    if raw_bytes > SOURCE_RECOVERY_MAX_BYTES {
        return Err("source recovery output exceeds the bounded store limit".to_owned());
    }
    let now = unix_now()?;
    let session_root = config.root.join("source-recovery").join(write.session_id);
    std::fs::create_dir_all(&session_root).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::set_permissions(&session_root, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    cleanup_expired_source_recoveries(&session_root, now);
    let recovery_root = session_root.join(write.token);
    std::fs::create_dir(&recovery_root).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::set_permissions(&recovery_root, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let record = SourceRecoveryMetadata {
        version: 1,
        source_execution_id: write.source_execution_id.to_string(),
        created_at_unix: now,
        expires_at_unix: now.saturating_add(SOURCE_RECOVERY_TTL.as_secs()),
        stdout_bytes: u64::try_from(write.stdout.len()).unwrap_or(u64::MAX),
        stderr_bytes: u64::try_from(write.stderr.len()).unwrap_or(u64::MAX),
    };
    let result = (|| {
        write_private_file(&recovery_root.join("stdout"), write.stdout)?;
        write_private_file(&recovery_root.join("stderr"), write.stderr)?;
        let encoded = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
        write_private_file(&recovery_root.join("metadata.json"), &encoded)
    })();
    if result.is_err() {
        let _removed = std::fs::remove_dir_all(&recovery_root);
    }
    result
}

pub(super) fn record_source_recovery(
    config: &Config,
    session_id: &str,
    metadata: &SourceRecoveryMetadata,
) -> Result<(), String> {
    let event = serde_json::json!({
        "session_id": session_id,
        "recovered_stdout_bytes": metadata.stdout_bytes,
        "recovered_stderr_bytes": metadata.stderr_bytes,
        "success": true,
    });
    let mut options = std::fs::OpenOptions::new();
    let _options = options.create(true).append(true);
    #[cfg(unix)]
    let _mode = options.mode(0o600);
    let mut output = options
        .open(config.root.join("source-recovery-events.jsonl"))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut output, &event).map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())
}

pub(super) fn recall_source_output(config: &Config, recovery_id: &str) -> Result<(), String> {
    if !valid_source_recovery_token(recovery_id) {
        return Err("invalid source recovery id".to_owned());
    }
    let session_id = source_session_id()
        .ok_or_else(|| "source recovery requires a valid Tracepress session".to_owned())?;
    let recovery_root = config
        .root
        .join("source-recovery")
        .join(&session_id)
        .join(recovery_id);
    let metadata: SourceRecoveryMetadata = serde_json::from_slice(
        &std::fs::read(recovery_root.join("metadata.json"))
            .map_err(|_error| "source recovery is unavailable".to_owned())?,
    )
    .map_err(|_error| "source recovery metadata is invalid".to_owned())?;
    if metadata.expires_at_unix < unix_now()? {
        let _removed = std::fs::remove_dir_all(&recovery_root);
        return Err("source recovery has expired".to_owned());
    }
    if metadata.stdout_bytes.saturating_add(metadata.stderr_bytes) > SOURCE_RECOVERY_MAX_BYTES {
        return Err("source recovery exceeds the bounded read limit".to_owned());
    }
    let stdout = std::fs::read(recovery_root.join("stdout"))
        .map_err(|_error| "source recovery stdout is unavailable".to_owned())?;
    let stderr = std::fs::read(recovery_root.join("stderr"))
        .map_err(|_error| "source recovery stderr is unavailable".to_owned())?;
    if u64::try_from(stdout.len()).unwrap_or(u64::MAX) != metadata.stdout_bytes
        || u64::try_from(stderr.len()).unwrap_or(u64::MAX) != metadata.stderr_bytes
    {
        return Err("source recovery byte counts do not match metadata".to_owned());
    }
    std::io::stdout()
        .write_all(&stdout)
        .map_err(|error| error.to_string())?;
    std::io::stderr()
        .write_all(&stderr)
        .map_err(|error| error.to_string())?;
    record_source_recovery(config, &session_id, &metadata)
}

pub(super) fn source_reducer_id(requested: Option<&str>) -> &'static str {
    requested
        .and_then(source_reducer_policy)
        .map_or("passthrough", |policy| policy.reducer_id)
}

pub(super) fn source_policy_decision(requested: Option<&str>) -> &'static str {
    let Some(requested) = requested else {
        return "passthrough";
    };
    match source_reducer_policy(requested).map(|policy| policy.decision) {
        Some(SourcePolicyDecision::ShadowOnly) => "shadow_only",
        Some(SourcePolicyDecision::AcceptedOptIn) => "accepted_opt_in",
        Some(SourcePolicyDecision::Rejected) => "rejected",
        Some(_) | None => "unknown",
    }
}

#[derive(Clone, Copy)]
pub(super) struct SourceReducerInput<'a> {
    requested: Option<&'a str>,
    family: CommandFamily,
    stdout: &'a [u8],
    stderr: &'a [u8],
}

pub(super) fn shadow_source_candidate(
    input: SourceReducerInput<'_>,
) -> Option<CargoOutputCandidate> {
    match (input.requested, input.family) {
        (Some("cargo_test_v1_shadow"), CommandFamily::CargoTest) => {
            Some(cargo_test_v1_shadow(input.stdout, input.stderr))
        }
        (Some("cargo_check_v1_shadow"), CommandFamily::CargoCheck) => {
            Some(cargo_check_v1_shadow(input.stdout, input.stderr))
        }
        (Some("cargo_check_v2_shadow"), CommandFamily::CargoCheck) => {
            Some(cargo_check_v2_shadow(input.stdout, input.stderr))
        }
        (Some("cargo_clippy_v1_shadow"), CommandFamily::CargoClippy) => {
            Some(cargo_clippy_v1_shadow(input.stdout, input.stderr))
        }
        (Some("rg_v1_shadow"), CommandFamily::Ripgrep) => {
            Some(rg_v1_shadow(input.stdout, input.stderr))
        }
        (Some("git_status_v1_shadow"), CommandFamily::GitStatus) => {
            Some(git_status_v1_shadow(input.stdout, input.stderr))
        }
        _ => None,
    }
}

pub(super) type ActiveSourceReducer = fn(&[u8], &[u8], &str) -> CargoOutputCandidate;

pub(super) fn active_source_reducer(
    requested: Option<&str>,
    family: CommandFamily,
) -> Option<ActiveSourceReducer> {
    let requested = requested?;
    if !active_source_policy_allowed(requested, family) {
        return None;
    }
    match (requested, family) {
        ("cargo_test_v1_active", CommandFamily::CargoTest) => Some(cargo_test_v1_active),
        ("cargo_check_v2_active", CommandFamily::CargoCheck) => Some(cargo_check_v2_active),
        ("rg_v1_active", CommandFamily::Ripgrep) => Some(rg_v1_active),
        _ => None,
    }
}

pub(super) fn source_policy_fail_open_reason(
    requested: Option<&str>,
    family: CommandFamily,
) -> Option<&'static str> {
    let requested = requested?;
    let Some(policy) = source_reducer_policy(requested) else {
        return Some("unknown_reducer");
    };
    if policy.command_family != family {
        return Some("policy_family_mismatch");
    }
    (policy.mode == SourcePolicyMode::Active && policy.decision == SourcePolicyDecision::Rejected)
        .then_some("policy_rejected")
}

#[derive(Clone, Copy)]
pub(super) struct ActiveSourceInput<'a> {
    config: &'a Config,
    metadata: &'a tracepress_tool_proxy::SourceExecutionMetadata,
    stdout: &'a [u8],
    stderr: &'a [u8],
}

pub(super) fn prepare_active_source_candidate(
    input: ActiveSourceInput<'_>,
    evaluate: impl FnOnce(&str) -> CargoOutputCandidate,
) -> (Option<CargoOutputCandidate>, bool, Option<&'static str>) {
    let mut candidate = None;
    let mut recovery_available = false;
    let attempt = (|| {
        let session_id = source_session_id().ok_or("missing_session")?;
        let token = source_recovery_token().map_err(|_error| "token_generation_failed")?;
        let executable = std::env::var("TRACEPRESS_TOOL_BIN")
            .ok()
            .filter(|value| safe_source_recovery_executable(value))
            .unwrap_or_else(|| "tracepress".to_owned());
        let recovery_command = format!("{executable} recall {token}");
        let evaluated = evaluate(&recovery_command);
        if !evaluated.never_worse_accepted {
            candidate = Some(evaluated);
            return Err("never_worse_rejected");
        }
        store_source_recovery(
            input.config,
            SourceRecoveryWrite {
                session_id: &session_id,
                token: &token,
                source_execution_id: input.metadata.source_execution_id,
                stdout: input.stdout,
                stderr: input.stderr,
            },
        )
        .map_err(|_error| "recovery_store_failed")?;
        candidate = Some(evaluated);
        recovery_available = true;
        Ok(())
    })();
    (candidate, recovery_available, attempt.err())
}

pub(super) fn tool(config: &Config, args: &[String]) -> Result<(), String> {
    let command = args.join(" ");
    let ids = UuidV7Generator::new();
    let (stdout, stderr, metadata) = execute_passthrough(&command, &ids)
        .map_err(|error| format!("source tool passthrough refused or failed: {error}"))?;
    let reducer = std::env::var("TRACEPRESS_SOURCE_REDUCER").ok();
    let mut candidate = shadow_source_candidate(SourceReducerInput {
        requested: reducer.as_deref(),
        family: metadata.command_family,
        stdout: &stdout,
        stderr: &stderr,
    });
    let mut active = false;
    let mut recovery_available = false;
    let mut fail_open_reason =
        source_policy_fail_open_reason(reducer.as_deref(), metadata.command_family);
    if let Some(evaluate) = active_source_reducer(reducer.as_deref(), metadata.command_family) {
        active = true;
        let active_fail_open_reason;
        (candidate, recovery_available, active_fail_open_reason) = prepare_active_source_candidate(
            ActiveSourceInput {
                config,
                metadata: &metadata,
                stdout: &stdout,
                stderr: &stderr,
            },
            |recovery_command| evaluate(&stdout, &stderr, recovery_command),
        );
        fail_open_reason = active_fail_open_reason.or(fail_open_reason);
    }
    let active_candidate = active
        .then_some(candidate.as_ref())
        .flatten()
        .filter(|evaluated| recovery_available && evaluated.never_worse_accepted);
    let emitted_stdout =
        active_candidate.map_or(stdout.as_slice(), |value| value.candidate_stdout.as_slice());
    let emitted_stderr =
        active_candidate.map_or(stderr.as_slice(), |value| value.candidate_stderr.as_slice());
    std::io::stdout()
        .write_all(emitted_stdout)
        .map_err(|error| error.to_string())?;
    std::io::stderr()
        .write_all(emitted_stderr)
        .map_err(|error| error.to_string())?;
    let emitted_bytes = u64::try_from(emitted_stdout.len().saturating_add(emitted_stderr.len()))
        .unwrap_or(u64::MAX);
    let reducer_id = source_reducer_id(reducer.as_deref());
    let policy_decision = source_policy_decision(reducer.as_deref());
    let _recorded = record_source_execution(
        config,
        &metadata,
        SourceEmissionRecord {
            candidate: candidate.as_ref(),
            reducer_id,
            policy_decision,
            emitted_bytes,
            active,
            recovery_available,
            fail_open_reason,
        },
    );
    if let Some(signal) = metadata.termination_signal {
        #[cfg(unix)]
        {
            let status = std::process::Command::new("/bin/kill")
                .args(["-s", &signal.to_string(), &std::process::id().to_string()])
                .status()
                .map_err(|error| format!("cannot reproduce child signal: {error}"))?;
            return Err(format!("signal reproducer unexpectedly returned: {status}"));
        }
        #[cfg(not(unix))]
        return Err(format!("child terminated by signal {signal}"));
    }
    if let Some(code) = metadata.exit_code
        && code != 0
    {
        std::process::exit(code);
    }
    Ok(())
}

pub(super) fn record_source_execution(
    config: &Config,
    metadata: &tracepress_tool_proxy::SourceExecutionMetadata,
    emission: SourceEmissionRecord<'_>,
) -> Result<(), String> {
    config.ensure_root()?;
    let session_id = source_session_id();
    let exit_status_class = if metadata.termination_signal.is_some() {
        "signal"
    } else if metadata.exit_code == Some(0) {
        "success"
    } else {
        "nonzero"
    };
    let command_family = match metadata.command_family {
        CommandFamily::CargoTest => "cargo_test",
        CommandFamily::CargoCheck => "cargo_check",
        CommandFamily::CargoClippy => "cargo_clippy",
        CommandFamily::Ripgrep => "rg",
        CommandFamily::GitStatus => "git_status",
        _ => "unknown",
    };
    let record = serde_json::json!({
        "source_execution_id": metadata.source_execution_id,
        "session_id": session_id,
        "command_family": command_family,
        "reducer_id": emission.reducer_id,
        "policy_decision": emission.policy_decision,
        "reducer_version": "v1",
        "output_contract": "agent_readable",
        "raw_stdout_bytes": metadata.raw_stdout_bytes,
        "raw_stderr_bytes": metadata.raw_stderr_bytes,
        "emitted_bytes": emission.emitted_bytes,
        "exit_status_class": exit_status_class,
        "duration_us": u64::try_from(metadata.duration.as_micros()).unwrap_or(u64::MAX),
        "recovery_available": emission.recovery_available,
        "active": emission.active,
        "forwarding_mutation": emission.active && emission.recovery_available,
        "fail_open_reason": emission.fail_open_reason,
        "shadow": emission.candidate.is_some() && !emission.active,
        "candidate_bytes": emission.candidate.map(|candidate| candidate.candidate_bytes),
        "estimated_raw_tokens": emission.candidate.map(|candidate| candidate.estimated_raw_tokens),
        "estimated_candidate_tokens": emission.candidate.map(|candidate| candidate.estimated_candidate_tokens),
        "candidate_applicable": emission.candidate.map(|candidate| candidate.applicable),
        "never_worse_accepted": emission.candidate.map(|candidate| candidate.never_worse_accepted),
        "omitted_passing_tests": emission.candidate.map(|candidate| candidate.omitted_passing_tests),
        "omitted_progress_lines": emission.candidate.map(|candidate| candidate.omitted_progress_lines),
        "omitted_advisory_lines": emission.candidate.map(|candidate| candidate.omitted_advisory_lines),
        "grouped_match_lines": emission.candidate.map(|candidate| candidate.grouped_match_lines),
        "recovery_hint_bytes": emission.candidate.map(|candidate| candidate.recovery_hint_bytes),
        "reducer_duration_us": emission.candidate.map(|candidate| {
            u64::try_from(candidate.reducer_duration.as_micros()).unwrap_or(u64::MAX)
        }),
    });
    let path = config.root.join("source-executions.jsonl");
    let mut options = std::fs::OpenOptions::new();
    let _options = options.create(true).append(true);
    #[cfg(unix)]
    let _mode = options.mode(0o600);
    let mut output = options.open(path).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut output, &record).map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())
}
