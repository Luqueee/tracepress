#![allow(
    clippy::multiple_crate_versions,
    clippy::match_wildcard_for_single_variants,
    clippy::unnecessary_wraps,
    clippy::use_debug,
    clippy::format_collect,
    clippy::indexing_slicing,
    clippy::map_unwrap_or,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::exit,
    clippy::redundant_pub_crate,
    clippy::unused_async,
    clippy::significant_drop_tightening,
    clippy::wildcard_imports,
    reason = "CLI boundary formats user-facing output, uses crate-scoped binary modules, and validates bounded fixed-size state"
)]
//! Tracepress command-line boundary.
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    future::Future,
    io::{BufRead as _, Read as _, Write as _},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use axum::serve;
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracepress_compression::{
    BlockKind as CompressionBlockKind, BlockMetadata, BlockOrigin as CompressionBlockOrigin,
    CacheRisk as CompressionCacheRisk, CandidateMetrics, CandidateStatus, CompressionLimits,
    DetectedKind as CompressionDetectedKind, JsonCompactRecords, JsonEmptyNoiseFieldReducer,
    JsonKeyElision, JsonMinify, JsonNoop, JsonReadableTable, JsonRepeatedSubtree,
    JsonRepeatedValueReducer, JsonTabular, ReductionMetrics, ReductionPolicyDecision,
    ReductionStatus, SearchResultReducer, ShadowCompressor, ShellDiagnosticProjectionReducer,
    ShellSemanticFamily, TextLogPrefixFold, TextNoop, TextReadableBlockFold, TextReadableLineFold,
    TextRepeatedLine, TextRepeatedRun, ToolFamily, ToolResultReducer, evaluate_with_estimator,
};
use tracepress_context::{
    ContextAnalysisLimits, ContextAnalysisResult, ContextAnalysisStatus, ContextBlockKind,
    ContextBlockSummary, ContextDeltaRequest, ContextOrigin, ContextRole, DetectedContentKind,
    EstimationRequest, MeasurementApplicability, StructuralHeuristicEstimator,
    TokenEstimateAggregate, TokenEstimator, TokenReconciliation, compute_context_delta,
};
use tracepress_core::{
    AttemptId, CompressionCandidateId, ContextSnapshotId, HttpStatusCode, MaxIpcFrameBytes,
    MaxRequestBodyBytes, MaxResponseBodyBytes, OperationId, RequestId, ResourceLimits,
    ResourceLimitsConfig, SessionId, SourceExecutionId, UuidV7Generator,
};
use tracepress_daemon::{
    ContextAnalysisFinalize, ContextAnalysisMetrics, ContextAppendReceipt,
    ContextCorrelationStatusWire, ControlRequest, ControlResponse, CorrelationDegradation,
    CorrelationStatus, ProviderObservation as ObservationRecord, ProviderObservationOutcome,
    ShadowExperimentCounters, ShadowExperimentManifest,
};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcLimits, IpcRequest, ResponseOutcome, UnixEndpoint,
};
use tracepress_provider::{
    ProviderEndpoint, ProviderRequestKind, ProviderResponseState, ProviderTransport,
    RequestObservation, ResponseObservation,
};
use tracepress_proxy::{
    ActiveCompressionMode, ActiveCompressionObservation, BackgroundTaskSpawner,
    CompactionObservation, ContextAnalysisDropReason, ContextAnalysisMode,
    ContextAnalysisObservation, ContextAnalysisOutcome, DeferredAnalysisMetrics, ForwardId,
    ForwardMetadata, InboundRoute, MetadataSink, MetadataSinkError, ObservationSinkError,
    ProviderObservationSink, ProxyConfig, RequestContextObservation, ShadowAnalysisBody,
    TransparentProxy, TransportFailure,
};
use tracepress_storage::{
    ContextInspection, ContextInspectionBlock, ContextInspectionNamedEstimate,
    ContextSnapshotStatus,
};
use tracepress_storage::{ShadowCacheRisk, ShadowCandidateRecord, ShadowCandidateStatus};
use tracepress_tool_proxy::{
    CargoOutputCandidate, CommandFamily, SourcePolicyDecision, SourcePolicyMode,
    active_source_policy_allowed, cargo_check_v1_shadow, cargo_check_v2_active,
    cargo_check_v2_shadow, cargo_clippy_v1_shadow, cargo_test_v1_active, cargo_test_v1_shadow,
    codex_pre_tool_use_identity_rewrite, codex_pre_tool_use_rewrite, execute_passthrough,
    git_status_v1_shadow, rg_v1_active, rg_v1_shadow, source_reducer_policy,
};

pub(crate) mod dashboard;
pub(crate) mod hooks;
pub(crate) mod inspection;
pub(crate) mod recording;
pub(crate) mod source;

use dashboard::*;
use hooks::*;
use inspection::*;
#[cfg(test)]
use recording::compression::shadow_candidate_batch_fits;
#[cfg(test)]
use recording::context::context_metrics;
use recording::context::flush_context_drops;
#[cfg(test)]
use recording::transport::{TransportDispatchJob, TransportOrdering, TransportSequence};
use recording::*;
use source::*;

const BODY_BYTES: u64 = 32_768;

#[derive(Debug, Parser)]
#[command(
    name = "tracepress",
    version,
    about = "Bounded Tracepress local runtime"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    Init,
    Doctor,
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Run {
        agent: String,
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    Context {
        request_id: RequestId,
    },
    /// Launches the local, read-only Tracepress Observatory.
    Ui {
        /// Local loopback port.
        #[arg(long, default_value_t = tracepress_dashboard_api::DEFAULT_PORT)]
        port: u16,
        /// Use synthetic metadata instead of the operational database.
        #[arg(long)]
        fixture: bool,
        /// Generate the 1,000-session / 10,000-request / 100,000-block smoke fixture.
        #[arg(long, requires = "fixture")]
        large_fixture: bool,
    },
    Proxy,
    /// Execute an admitted source-side tool command without reduction.
    Tool {
        #[arg(required = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Recover byte-faithful source output hidden by an active reducer.
    Recall {
        recovery_id: String,
    },
    /// Process a fail-open Codex hook payload from standard input.
    Hook {
        agent: String,
    },
    /// Perform a metadata-only Codex app-server handshake; it never starts a thread or turn.
    CodexObserve {
        /// Start one ephemeral read-only turn that runs a harmless printf command.
        #[arg(long)]
        command_smoke: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    Start,
    Stop,
    Status,
}

#[derive(Clone, Debug)]
struct Config {
    root: PathBuf,
    database: PathBuf,
    socket: PathBuf,
    credential: PathBuf,
    ready: PathBuf,
}

/// Session-owned Codex hook home. It is removed once the child process exits.
#[derive(Debug)]
struct TemporaryCodexHookHome(PathBuf);

const CODEX_HOOK_HOME_GRACE: Duration = Duration::from_secs(60 * 60);

/// Removes only expired UUID-shaped children of Tracepress' dedicated hook root.
fn cleanup_stale_codex_hook_homes(root: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let uuid_shaped =
            name.len() == 36 && name.bytes().filter(|byte| *byte == b'-').count() == 4;
        let expired = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > CODEX_HOOK_HOME_GRACE);
        if uuid_shaped && expired {
            let _removed = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn temporary_codex_hook_home(
    config: &Config,
    session_id: SessionId,
) -> Result<TemporaryCodexHookHome, String> {
    let source_root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or_else(|| {
            "cannot locate Codex home for temporary session authentication".to_owned()
        })?;
    let auth = source_root.join("auth.json");
    let sessions_root = config.root.join("codex-hook-sessions");
    std::fs::create_dir_all(&sessions_root).map_err(|error| error.to_string())?;
    cleanup_stale_codex_hook_homes(&sessions_root);
    let root = sessions_root.join(session_id.to_string());
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let _copied = std::fs::copy(&auth, root.join("auth.json"))
        .map_err(|error| format!("cannot copy temporary Codex session authentication: {error}"))?;
    #[cfg(unix)]
    std::fs::set_permissions(
        root.join("auth.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let shim_dir = root.join("bin");
    std::fs::create_dir(&shim_dir).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&executable, shim_dir.join("cargo"))
        .map_err(|error| error.to_string())?;
    let command = format!("{} hook codex", executable.display());
    let hooks = serde_json::json!({"hooks":{"PreToolUse":[{"matcher":"^Bash$","hooks":[{"type":"command","command":command,"timeout":5}]}]}});
    std::fs::write(
        root.join("hooks.json"),
        serde_json::to_vec_pretty(&hooks).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(TemporaryCodexHookHome(root))
}

impl TemporaryCodexHookHome {
    fn cleanup(self) {
        let _removed = std::fs::remove_dir_all(self.0);
    }
}

impl Config {
    fn load() -> Result<Self, String> {
        let root = std::env::var_os("TRACEPRESS_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".tracepress"));
        Ok(Self {
            database: root.join("tracepress.sqlite3"),
            socket: root.join("tracepress.sock"),
            credential: root.join("control.cred"),
            ready: root.join("daemon.ready"),
            root,
        })
    }
    fn ensure_root(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("cannot create {}: {e}", self.root.display()))?;
        #[cfg(unix)]
        std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot secure {}: {e}", self.root.display()))?;
        Ok(())
    }
}

fn limits() -> Result<IpcLimits, String> {
    Ok(IpcLimits::new(
        MaxIpcFrameBytes::new(FRAME_BYTES).map_err(|e| e.to_string())?,
        MaxRequestBodyBytes::new(BODY_BYTES).map_err(|e| e.to_string())?,
        MaxResponseBodyBytes::new(BODY_BYTES).map_err(|e| e.to_string())?,
    ))
}
fn credential(config: &Config) -> Result<Credential, String> {
    let text = std::fs::read_to_string(&config.credential)
        .map_err(|e| format!("cannot read credential: {e}"))?;
    let text = text.trim();
    if text.len() != 64 {
        return Err("credential must contain exactly 32 bytes".to_owned());
    }
    let (pairs, remainder) = text.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err("credential must contain exactly 32 bytes".to_owned());
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in pairs.iter().enumerate() {
        bytes[index] = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| "credential is not UTF-8")?,
            16,
        )
        .map_err(|_| "credential is not hexadecimal")?;
    }
    Ok(Credential::new(bytes))
}

async fn control(config: &Config, request: ControlRequest) -> Result<ControlResponse, String> {
    let client = IpcClient::authenticated(
        Endpoint::Unix(UnixEndpoint::new(config.socket.clone()).map_err(|e| e.to_string())?),
        credential(config)?,
        limits()?,
    );
    let cancellation = CancellationToken::new();
    let mut connection = client
        .connect(&cancellation)
        .await
        .map_err(|e| format!("daemon unavailable: {e}"))?;
    let ids = UuidV7Generator::new();
    let body = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let request = IpcRequest::new(
        RequestId::generate(&ids),
        body,
        MaxRequestBodyBytes::new(BODY_BYTES).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    connection
        .send_request(&request, &cancellation)
        .await
        .map_err(|e| e.to_string())?;
    let response = connection
        .receive_response(&cancellation)
        .await
        .map_err(|e| e.to_string())?;
    match response.outcome() {
        ResponseOutcome::Complete { body } => {
            serde_json::from_slice(body).map_err(|e| format!("invalid daemon response: {e}"))
        }
        ResponseOutcome::Incomplete | ResponseOutcome::Cancelled => {
            Err("daemon returned an incomplete response".to_owned())
        }
    }
}

/// Reconciles pending analyses against durable daemon state before the forced-drain drop
/// boundary. A status without a completion timestamp is still active and must remain pending.
async fn reconcile_pending_context(config: &Config, counters: &ContextCounters) {
    reconcile_pending_context_with(counters, |evidence| async move {
        let request = ControlRequest::ContextStatus {
            request_id: Some(evidence.provider_request_id),
            snapshot_id: evidence.snapshot_id,
        };
        match tokio::time::timeout(Duration::from_millis(250), control(config, request)).await {
            Ok(Ok(ControlResponse::ContextStatus {
                snapshot_status: Some(status),
            })) => Some(status),
            _ => None,
        }
    })
    .await;
}

async fn reconcile_pending_context_with<F, Fut>(counters: &ContextCounters, mut lookup: F)
where
    F: FnMut(PendingAnalysisEvidence) -> Fut,
    Fut: Future<Output = Option<ContextSnapshotStatus>>,
{
    for evidence in counters.pending_evidence() {
        let Some(status) = lookup(evidence).await else {
            continue;
        };
        if status.completed_at_us.is_none() {
            continue;
        }
        if status.status == "complete" {
            counters.complete_sequence(evidence.sequence);
        } else {
            counters.partial_sequence(evidence.sequence);
        }
    }
}

async fn daemon_running(config: &Config) -> Result<bool, String> {
    if !config.socket.exists() {
        return Ok(false);
    }
    match tokio::net::UnixStream::connect(&config.socket).await {
        Ok(probe) => drop(probe),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(format!("cannot probe daemon socket: {error}")),
    }
    match control(config, ControlRequest::Status).await? {
        ControlResponse::Ok { .. } => Ok(true),
        ControlResponse::Error { message } => Err(message),
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
            Err("daemon returned an unexpected context response".to_owned())
        }
    }
}
async fn init(config: &Config) -> Result<(), String> {
    config.ensure_root()?;
    if !config.credential.exists() {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut options = std::fs::OpenOptions::new();
        let _options = options.write(true).create_new(true);
        #[cfg(unix)]
        let _mode = options.mode(0o600);
        std::io::Write::write_all(
            &mut options
                .open(&config.credential)
                .map_err(|e| e.to_string())?,
            hex.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    }
    println!("initialized {}", config.root.display());
    Ok(())
}

async fn daemon_start(config: &Config) -> Result<(), String> {
    init(config).await?;
    if daemon_running(config).await? {
        return Err("daemon is already running".to_owned());
    }
    let daemon = std::env::var_os("TRACEPRESSD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tracepressd"));
    let mut child = Command::new(daemon)
        .env("TRACEPRESS_DATABASE", &config.database)
        .env("TRACEPRESS_CONTROL_SOCKET", &config.socket)
        .env("TRACEPRESS_CONTROL_CREDENTIAL", &config.credential)
        .env("TRACEPRESS_DAEMON_READY", &config.ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start tracepressd: {e}"))?;
    for _ in 0..50 {
        if config.ready.exists() {
            match daemon_running(config).await {
                Ok(true) => {
                    println!("daemon running");
                    return Ok(());
                }
                Ok(false) => {}
                Err(error) => {
                    let _killed = child.kill().await;
                    return Err(format!("daemon readiness check failed: {error}"));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = child.kill().await;
    Err("daemon did not become ready within 5 seconds".to_owned())
}

async fn daemon_stop(config: &Config) -> Result<(), String> {
    let response = control(config, ControlRequest::Shutdown).await?;
    match response {
        ControlResponse::Ok { .. } => {}
        ControlResponse::Error { message } => return Err(message),
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    }
    for _ in 0..50 {
        if !config.socket.exists() {
            let _removed = std::fs::remove_file(&config.ready);
            println!("daemon stopped");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("daemon did not stop within 5 seconds".to_owned())
}

fn proxy_resource_limits(
    request_bytes: u64,
    response_bytes: u64,
) -> Result<ResourceLimits, String> {
    ResourceLimits::try_from(ResourceLimitsConfig {
        max_raw_bytes: Some(i128::from(request_bytes)),
        max_request_body_bytes: Some(i128::from(request_bytes)),
        max_response_body_bytes: Some(i128::from(response_bytes)),
        max_decompressed_bytes: Some(i128::from(response_bytes)),
        max_ipc_frame_bytes: Some(i128::from(FRAME_BYTES)),
        max_ipc_queue_items: Some(i128::from(RECORDER_QUEUE_ITEMS as u64)),
        max_json_nesting: Some(64),
        max_json_items: Some(100_000),
        max_line_bytes: Some(i128::from(request_bytes)),
        max_processing_time_ms: Some(250),
        max_cpu_work_units: Some(1_000_000),
    })
    .map_err(|error| error.to_string())
}

fn context_analysis_mode() -> Result<ContextAnalysisMode, String> {
    let value = std::env::var("TRACEPRESS_CONTEXT_ANALYSIS")
        .unwrap_or_else(|_| "shadow".to_owned())
        .to_ascii_lowercase();
    match value.as_str() {
        "off" => Ok(ContextAnalysisMode::Off),
        "shadow" => Ok(ContextAnalysisMode::Shadow),
        _ => Err(format!(
            "TRACEPRESS_CONTEXT_ANALYSIS must be `off` or `shadow`, got `{value}`"
        )),
    }
}

fn shadow_compression_enabled() -> Result<bool, String> {
    let value = std::env::var("TRACEPRESS_SHADOW_COMPRESSION")
        .unwrap_or_else(|_| "off".to_owned())
        .to_ascii_lowercase();
    match value.as_str() {
        "off" => Ok(false),
        "on" => Ok(true),
        _ => Err(format!(
            "TRACEPRESS_SHADOW_COMPRESSION must be `off` or `on`, got `{value}`"
        )),
    }
}

fn active_compression_mode() -> Result<ActiveCompressionMode, String> {
    let value = std::env::var("TRACEPRESS_ACTIVE_COMPRESSION")
        .unwrap_or_else(|_| "off".to_owned())
        .to_ascii_lowercase();
    match value.as_str() {
        "off" => Ok(ActiveCompressionMode::Off),
        "json.minify" | "json_minify" => Ok(ActiveCompressionMode::JsonMinify),
        "search.result_projection" | "search_projection" => {
            Ok(ActiveCompressionMode::SearchProjection)
        }
        _ => Err(format!(
            "TRACEPRESS_ACTIVE_COMPRESSION must be `off`, `json.minify`, or `search.result_projection`, got `{value}`"
        )),
    }
}

fn shadow_experiment_id() -> Result<String, String> {
    let value = std::env::var("TRACEPRESS_SHADOW_EXPERIMENT_ID")
        .unwrap_or_else(|_| "shadow-pilot-001".to_owned());
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(value)
    } else {
        Err("TRACEPRESS_SHADOW_EXPERIMENT_ID must be 1-128 ASCII identifier characters".to_owned())
    }
}

fn is_codex_agent(agent: &str) -> bool {
    std::path::Path::new(agent)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "codex" || name == "codex.exe")
}

fn provider_transport_from_value(value: &str) -> Result<ProviderTransport, String> {
    match value.to_ascii_lowercase().as_str() {
        "openai_public_api" | "public" => Ok(ProviderTransport::OpenAiPublicApi),
        "chatgpt_codex_subscription" | "subscription" | "codex_subscription" => {
            Ok(ProviderTransport::ChatGptCodexSubscription)
        }
        _ => Err(format!(
            "TRACEPRESS_PROVIDER_TRANSPORT must be `openai_public_api` or `chatgpt_codex_subscription`, got `{value}`"
        )),
    }
}

fn provider_transport_for_agent(agent: &str) -> Result<ProviderTransport, String> {
    if let Ok(value) = std::env::var("TRACEPRESS_PROVIDER_TRANSPORT") {
        return provider_transport_from_value(&value);
    }
    if is_codex_agent(agent) {
        Ok(ProviderTransport::ChatGptCodexSubscription)
    } else {
        Ok(ProviderTransport::OpenAiPublicApi)
    }
}

fn provider_endpoint(transport: ProviderTransport) -> Result<ProviderEndpoint, String> {
    match transport {
        ProviderTransport::OpenAiPublicApi => {
            let upstream = std::env::var("TRACEPRESS_UPSTREAM")
                .map_err(|_| "TRACEPRESS_UPSTREAM is required by `tracepress run`".to_owned())?;
            ProviderEndpoint::new(&upstream).map_err(|error| error.to_string())
        }
        ProviderTransport::ChatGptCodexSubscription => {
            if std::env::var_os("TRACEPRESS_UPSTREAM").is_some() {
                return Err(
                    "TRACEPRESS_UPSTREAM cannot override the fixed ChatGPT Codex subscription endpoint"
                        .to_owned(),
                );
            }
            Ok(ProviderEndpoint::chatgpt_codex_subscription())
        }
        _ => Err("unsupported provider transport".to_owned()),
    }
}

const MEASUREMENT_METADATA_PREFIX: &str = "TRACEPRESS_MEASUREMENT_METADATA=";
const SCHEDULER_METRICS_PREFIX: &str = "TRACEPRESS_SCHEDULER_METRICS=";

fn measurement_metadata_line(
    measurement_run_id: Option<&str>,
    session_id: SessionId,
    started_at: &str,
) -> Option<String> {
    let measurement_run_id =
        measurement_run_id.filter(|value| !value.is_empty() && value.len() <= 128)?;
    let metadata = serde_json::json!({
        "measurement_run_id": measurement_run_id,
        "session_id": session_id.to_string(),
        "tracepress_pid": std::process::id(),
        "started_at": started_at,
    });
    Some(format!(
        "{MEASUREMENT_METADATA_PREFIX}{}",
        serde_json::to_string(&metadata).ok()?
    ))
}

fn scheduler_metrics_line(metrics: &DeferredAnalysisMetrics) -> String {
    let metrics = serde_json::json!({
        "deferred_queue_items": metrics.queue_items,
        "deferred_queue_bytes": metrics.queue_bytes,
        "deferred_high_water_items": metrics.high_water_items,
        "deferred_high_water_bytes": metrics.high_water_bytes,
        "analysis_admitted_total": metrics.deferred_total,
        "analysis_deferred_total": metrics.deferred_due_to_active_forwards_total,
        "processed_deferred_total": metrics.processed_deferred_total,
        "backlog_capacity_drops": metrics.backlog_capacity_drops,
        "analysis_wait_us": metrics.analysis_wait_us,
    });
    format!("{SCHEDULER_METRICS_PREFIX}{metrics}")
}

fn configure_codex_subscription(args: &mut Vec<String>, proxy_address: std::net::SocketAddr) {
    let base_url = format!("http://{proxy_address}/v1");
    let overrides = [
        "-c".to_owned(),
        "model_provider=tracepress_subscription".to_owned(),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.name=OpenAI".to_owned(),
        "-c".to_owned(),
        format!("model_providers.tracepress_subscription.base_url=\"{base_url}\""),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.wire_api=\"responses\"".to_owned(),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.requires_openai_auth=true".to_owned(),
        "-c".to_owned(),
        "model_providers.tracepress_subscription.supports_websockets=false".to_owned(),
    ];
    let insertion = args
        .iter()
        .position(|argument| argument == "exec")
        .map_or(0, |index| index.saturating_add(1));
    let _removed = args.splice(insertion..insertion, overrides);
}

#[allow(
    clippy::too_many_lines,
    reason = "the existing run path keeps lifecycle cleanup and reporting ordered"
)]
async fn run_agent(config: &Config, agent: String, args: Vec<String>) -> Result<(), String> {
    if !daemon_running(config).await? {
        return Err("daemon is not running; run `tracepress daemon start` first".to_owned());
    }
    let transport = provider_transport_for_agent(&agent)?;
    let endpoint = provider_endpoint(transport)?;
    let resource_limits = proxy_resource_limits(8 * 1024 * 1024, 32 * 1024 * 1024)?;
    let analysis_mode = context_analysis_mode()?;
    let shadow_enabled = shadow_compression_enabled()?;
    let active_mode = active_compression_mode()?;
    if shadow_enabled && !matches!(analysis_mode, ContextAnalysisMode::Shadow) {
        return Err("shadow compression requires TRACEPRESS_CONTEXT_ANALYSIS=shadow".to_owned());
    }
    if !matches!(active_mode, ActiveCompressionMode::Off)
        && !matches!(analysis_mode, ContextAnalysisMode::Shadow)
    {
        return Err("active compression requires TRACEPRESS_CONTEXT_ANALYSIS=shadow".to_owned());
    }
    let shadow_experiment_id = shadow_experiment_id()?;
    let context_queue_items = usize::try_from(resource_limits.max_ipc_queue_items.get())
        .map_err(|error| format!("context queue capacity does not fit usize: {error}"))?;
    let context_analysis_limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)
        .map_err(|error| error.to_string())?;
    let proxy = TransparentProxy::new(
        ProxyConfig::new(endpoint, resource_limits, analysis_mode)
            .map(|config| config.with_active_compression_mode(active_mode))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let proxy_address = listener.local_addr().map_err(|error| error.to_string())?;
    let started_at = current_timestamp()?;
    let response = control(
        config,
        ControlRequest::StartSession {
            started_at: started_at.clone(),
        },
    )
    .await?;
    let (session, parent_operation_id) = match response {
        ControlResponse::Ok {
            session: Some(session),
            operation_id: Some(operation_id),
            ..
        } => (session, operation_id),
        ControlResponse::Error { message } => return Err(message),
        ControlResponse::Ok { .. } => {
            return Err("daemon did not return a session and root operation".to_owned());
        }
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    };
    if let Some(metadata) = measurement_metadata_line(
        std::env::var("TRACEPRESS_MEASUREMENT_RUN_ID")
            .ok()
            .as_deref(),
        session.session_id,
        &started_at,
    ) {
        println!("{metadata}");
    }
    let SpawnedRecorder {
        proxy,
        mut recorder_task,
        mut transport_task,
        mut context_task,
        mut shadow_task,
        counters,
        context_counters,
        active_counters,
    } = spawn_recorder(
        RunRecording {
            config: config.clone(),
            session_id: session.session_id,
            parent_operation_id,
            context_queue_items,
            context_analysis_limits,
            analysis_enabled: matches!(analysis_mode, ContextAnalysisMode::Shadow),
            shadow_enabled,
            shadow_experiment_id,
            previous_context: None,
        },
        proxy,
    );
    let background_proxy = proxy.clone();
    let proxy_task = tokio::spawn(async move { serve(listener, proxy.router()).await });
    let base_url = format!("http://{proxy_address}/v1");
    let mut command = Command::new(&agent);
    let mut args = args;
    if matches!(transport, ProviderTransport::ChatGptCodexSubscription) && is_codex_agent(&agent) {
        configure_codex_subscription(&mut args, proxy_address);
    }
    let source_hook = std::env::var("TRACEPRESS_SOURCE_HOOK").ok();
    let hook_home = (is_codex_agent(&agent)
        && matches!(source_hook.as_deref(), Some("codex" | "path")))
    .then(|| temporary_codex_hook_home(config, session.session_id))
    .transpose()?;
    if let Some(home) = hook_home.as_ref() {
        // This applies only to the Tracepress-generated, session-owned hook home. It does not
        // alter sandbox or approval policy; Codex still evaluates the rewritten command normally.
        args.insert(0, "--dangerously-bypass-hook-trust".to_owned());
        let _home = command.env("CODEX_HOME", &home.0);
        let _binary = command.env(
            "TRACEPRESS_TOOL_BIN",
            std::env::current_exe().map_err(|error| error.to_string())?,
        );
        if source_hook.as_deref() == Some("path") {
            let original_path = std::env::var_os("PATH").unwrap_or_default();
            let real_cargo = std::env::split_paths(&original_path)
                .map(|directory| directory.join("cargo"))
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| "cannot locate Cargo before PATH interception".to_owned())?;
            let joined = std::env::join_paths([home.0.join("bin"), PathBuf::from(original_path)])
                .map_err(|error| error.to_string())?;
            let _path = command.env("PATH", joined);
            let _cargo = command.env("TRACEPRESS_REAL_CARGO", real_cargo);
            let _observe = command.env("TRACEPRESS_HOOK_REWRITE_MODE", "observe");
        }
    }
    let status_result = command
        .args(args)
        .env("TRACEPRESS_SESSION_ID", session.session_id.to_string())
        .env("OPENAI_BASE_URL", &base_url)
        .env(
            "TRACEPRESS_PROXY_URL",
            format!("http://{proxy_address}/v1/chat/completions"),
        )
        .env(
            "TRACEPRESS_RESPONSES_URL",
            format!("http://{proxy_address}/v1/responses"),
        )
        .status()
        .await;
    if let Some(home) = hook_home {
        home.cleanup();
    }
    proxy_task.abort();
    let _proxy_result = proxy_task.await;
    let (recorded, drain_timed_out) = match tokio::time::timeout(RECORDER_DRAIN_TIMEOUT, async {
        background_proxy.wait_for_background_tasks().await;
        // The proxy task was aborted above, so this is the scheduler's final drain barrier: no
        // new forwards can mutate these counters while recorder/context evidence is drained.
        let deferred_metrics = background_proxy.deferred_analysis_metrics();
        println!(
            "deferred_queue_items={}\ndeferred_queue_bytes={}\ndeferred_high_water_items={}\ndeferred_high_water_bytes={}\nanalysis_admitted_total={}\nanalysis_deferred_total={}\nprocessed_deferred_total={}\nbacklog_capacity_drops={}\nanalysis_wait_us={}",
            deferred_metrics.queue_items,
            deferred_metrics.queue_bytes,
            deferred_metrics.high_water_items,
            deferred_metrics.high_water_bytes,
            deferred_metrics.deferred_total,
            deferred_metrics.deferred_due_to_active_forwards_total,
            deferred_metrics.processed_deferred_total,
            deferred_metrics.backlog_capacity_drops,
            deferred_metrics.analysis_wait_us,
        );
        // The recorder's channel must be closed before it is drained; otherwise the recorder
        // cannot observe end-of-run and wait for more events forever.
        drop(background_proxy);
        let drain_result =
            drain_recorders(
                &mut recorder_task,
                &mut transport_task,
                &mut context_task,
                &mut shadow_task,
            )
            .await;
        // Emit the machine-readable scheduler snapshot only after recorder/context drain. The
        // collector publishes it only after the child exits, binding it to this run identity.
        println!("{}", scheduler_metrics_line(&deferred_metrics));
        drain_result
    })
    .await
    {
        Ok(result) => (result, false),
        Err(_elapsed) => {
            // Reconcile any snapshot that committed before its response was lost. Active
            // snapshots are intentionally left pending until FinishSession gives the daemon a
            // chance to finalize them as durable partial outcomes.
            reconcile_pending_context(config, &context_counters).await;
            transport_task.abort();
            recorder_task.abort();
            context_task.abort();
            if let Some(task) = shadow_task.as_mut() {
                task.abort();
            }
            // The cancelled drain future may already have consumed one or more task outputs.
            // Polling those JoinHandles again panics, so abort every remaining worker and let
            // their handles drop without a second poll.
            (
                Err(format!(
                    "provider observer/recorder drain exceeded {}s",
                    RECORDER_DRAIN_TIMEOUT.as_secs()
                )),
                true,
            )
        }
    };
    let ended_at = current_timestamp()?;
    let finalization = control(
        config,
        ControlRequest::FinishSession {
            session_id: session.session_id,
            ended_at,
        },
    )
    .await;
    if drain_timed_out {
        // FinishSession may have finalized active snapshots as durable partials. Reconcile only
        // after that boundary, then classify the genuinely unresolved remainder as dropped.
        reconcile_pending_context(config, &context_counters).await;
        context_counters.drop_pending(ContextAnalysisDropReason::Cancelled);
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            flush_context_drops(config, session.session_id, &context_counters),
        )
        .await;
    }
    println!("{}", counters.report());
    println!("{}", context_counters.report());
    println!("{}", active_counters.report());
    let status = status_result.map_err(|error| format!("cannot launch agent {agent}: {error}"))?;
    recorded.map_err(|error| format!("forward recording failed: {error}"))?;
    match finalization? {
        ControlResponse::Ok { .. } => {}
        ControlResponse::Error { message } => {
            return Err(format!(
                "agent finished but session finalization failed: {message}"
            ));
        }
        ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    }
    match status.code() {
        Some(code) if code != 0 => Err(format!("agent exited with status {code}")),
        Some(_) => Ok(()),
        None => Err("agent terminated by signal".to_owned()),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "each independently owned worker is drained explicitly"
)]
async fn drain_recorders(
    recorder_task: &mut tokio::task::JoinHandle<Result<(), String>>,
    transport_task: &mut tokio::task::JoinHandle<()>,
    context_task: &mut tokio::task::JoinHandle<Result<(), String>>,
    shadow_task: &mut Option<tokio::task::JoinHandle<Result<(), String>>>,
) -> Result<(), String> {
    let recorder_result = (&mut *recorder_task)
        .await
        .map_err(|error| format!("recording worker failed: {error}"))
        .and_then(|result| result);
    let _transport_result = (&mut *transport_task).await;
    // Always wait for the context worker after the recorder has stopped, even when the
    // provider worker reported an IPC error.
    let context_result = (&mut *context_task)
        .await
        .map_err(|error| format!("context worker failed: {error}"))
        .and_then(|result| result);
    let shadow_result = if let Some(task) = shadow_task.as_mut() {
        task.await
            .map_err(|error| format!("shadow worker failed: {error}"))
            .and_then(|result| result)
    } else {
        Ok(())
    };
    recorder_result.and(context_result).and(shadow_result)
}

fn current_timestamp() -> Result<String, String> {
    Ok(format!(
        "unix-ms:{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis()
    ))
}
async fn proxy() -> Result<(), String> {
    let transport = std::env::var("TRACEPRESS_PROVIDER_TRANSPORT")
        .map_or(Ok(ProviderTransport::OpenAiPublicApi), |value| {
            provider_transport_from_value(&value)
        })?;
    let endpoint = provider_endpoint(transport)?;
    let resource_limits = proxy_resource_limits(8 * 1024 * 1024, 32 * 1024 * 1024)?;
    let analysis_mode = context_analysis_mode()?;
    let active_mode = active_compression_mode()?;
    if !matches!(active_mode, ActiveCompressionMode::Off)
        && !matches!(analysis_mode, ContextAnalysisMode::Shadow)
    {
        return Err("active compression requires TRACEPRESS_CONTEXT_ANALYSIS=shadow".to_owned());
    }
    let proxy = TransparentProxy::new(
        ProxyConfig::new(endpoint, resource_limits, analysis_mode)
            .map(|config| config.with_active_compression_mode(active_mode))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listen =
        std::env::var("TRACEPRESS_PROXY_LISTEN").unwrap_or_else(|_| "127.0.0.1:0".to_owned());
    let listener = TcpListener::bind(&listen)
        .await
        .map_err(|error| error.to_string())?;
    println!(
        "proxy listening on {}",
        listener.local_addr().map_err(|error| error.to_string())?
    );
    let background_proxy = proxy.clone();
    let serve_result = serve(listener, proxy.router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| error.to_string());
    let drain_result = tokio::time::timeout(
        RECORDER_DRAIN_TIMEOUT,
        background_proxy.wait_for_background_tasks(),
    )
    .await;
    if let Err(error) = serve_result {
        let _ = drain_result;
        return Err(error);
    }
    match drain_result {
        Ok(()) => Ok(()),
        Err(_elapsed) => {
            let metrics = background_proxy.deferred_analysis_metrics();
            Err(format!(
                "proxy observer drain exceeded {}s (queue_items={}, queue_bytes={})",
                RECORDER_DRAIN_TIMEOUT.as_secs(),
                metrics.queue_items,
                metrics.queue_bytes,
            ))
        }
    }
}

fn current_timestamp_us() -> Result<u64, String> {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_micros(),
    )
    .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let invoked_as_cargo = std::env::args_os()
        .next()
        .and_then(|value| PathBuf::from(value).file_name().map(|name| name == "cargo"))
        .unwrap_or(false);
    let config = Config::load()?;
    if invoked_as_cargo {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        let mut tool_args = vec!["cargo".to_owned()];
        tool_args.extend(args);
        return tool(&config, &tool_args);
    }
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Init => init(&config).await,
        CommandKind::Doctor => doctor(&config).await,
        CommandKind::Daemon {
            command: DaemonCommand::Start,
        } => daemon_start(&config).await,
        CommandKind::Context { request_id } => context(&config, request_id).await,
        CommandKind::Ui {
            port,
            fixture,
            large_fixture,
        } => {
            dashboard(
                &config,
                DashboardOptions {
                    port,
                    fixture,
                    large_fixture,
                },
            )
            .await
        }
        CommandKind::Daemon {
            command: DaemonCommand::Stop,
        } => daemon_stop(&config).await,
        CommandKind::Daemon {
            command: DaemonCommand::Status,
        } => {
            println!(
                "{}",
                if daemon_running(&config).await? {
                    "running"
                } else {
                    "stopped"
                }
            );
            Ok(())
        }
        CommandKind::Run { agent, args } => run_agent(&config, agent, args).await,
        CommandKind::Proxy => proxy().await,
        CommandKind::Tool { args } => tool(&config, &args),
        CommandKind::Recall { recovery_id } => recall_source_output(&config, &recovery_id),
        CommandKind::Hook { agent } => hook(&agent),
        CommandKind::CodexObserve { command_smoke } => codex_observe(command_smoke),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the bounded experimental observer keeps its protocol lifecycle in one auditable scope"
)]
#[cfg(test)]
mod tests;
