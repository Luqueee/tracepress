#![allow(
    clippy::multiple_crate_versions,
    clippy::match_wildcard_for_single_variants,
    clippy::unnecessary_wraps,
    clippy::use_debug,
    clippy::format_collect,
    clippy::indexing_slicing,
    clippy::map_unwrap_or,
    clippy::print_stdout,
    clippy::unused_async,
    reason = "CLI boundary formats user-facing output and validates bounded fixed-size state"
)]
//! Tracepress command-line boundary.
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::serve;
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracepress_core::{
    HttpStatusCode, MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes, OperationId,
    RequestId, SessionId, UuidV7Generator,
};
use tracepress_daemon::{
    ControlRequest, ControlResponse, CorrelationDegradation, CorrelationStatus,
    ProviderObservation as ObservationRecord, ProviderObservationOutcome,
};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcLimits, IpcRequest, ResponseOutcome, UnixEndpoint,
};
use tracepress_provider::{
    ProviderEndpoint, ProviderResponseState, RequestObservation, ResponseObservation,
};
use tracepress_proxy::{
    ForwardId, ForwardMetadata, InboundRoute, MetadataSink, MetadataSinkError,
    ObservationSinkError, ProviderObservationSink, ProxyConfig, TransparentProxy, TransportFailure,
};

const FRAME_BYTES: u64 = 65_536;

/// Bounded queue of transport and semantic records awaiting durable recording.
const RECORDER_QUEUE_ITEMS: usize = 128;

/// Bound on forwards whose transport and semantic halves are still being correlated.
const IN_FLIGHT_FORWARDS: usize = 64;

/// Bound on retired identities kept one by one above the retirement watermark.
///
/// The watermark absorbs the contiguous prefix of retired identities for free, so this bound is
/// only reached when that many identities the proxy never reported an event for sit below the
/// newest retirement. Beyond it the oldest retirement is forgotten, which is the one least
/// likely to still have a half in flight.
const RETIRED_IDENTITIES: usize = 256;

/// Maximum wait for the recorder to drain after the agent exits.
const RECORDER_DRAIN_SECONDS: u64 = 5;

/// Correlation accounting of one run, published while the recorder is still working.
///
/// The recorder owns bounded state, so evidence it cannot join is lost by design. Every counter
/// here names one such loss, and they live behind a shared handle so the run can report them
/// even when the bounded drain window expired before the recorder finished.
#[derive(Debug, Default)]
struct CorrelationCounters {
    /// Every degradation of this run, whatever its reason.
    degraded_total: AtomicU64,
    /// Correlation state evicted before settling because the in-flight bound was reached.
    degraded_inflight_limit: AtomicU64,
    /// Halves refused because the bounded retirement memory still accounts for their identity.
    degraded_retired_limit: AtomicU64,
    /// Terminal semantic evidence that reached the recorder without its request half.
    missing_total: AtomicU64,
}

impl CorrelationCounters {
    /// Counts one degradation under its reason and in the run total.
    fn degraded(&self, reason: CorrelationDegradation) {
        let counter = match reason {
            CorrelationDegradation::InFlightLimit => Some(&self.degraded_inflight_limit),
            CorrelationDegradation::RetiredLimit => Some(&self.degraded_retired_limit),
            CorrelationDegradation::MissingRequestHalf => Some(&self.missing_total),
            // A reason this build keeps no counter for still counts in the run total.
            _ => None,
        };
        if let Some(counter) = counter {
            let _counted = counter.fetch_add(1, Ordering::Relaxed);
        }
        let _total = self.degraded_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Reports every counter, including the zeroes.
    ///
    /// A run reports its correlation accounting unconditionally: a line printed only when
    /// something degraded cannot distinguish a clean run from a report that never arrived.
    fn report(&self) -> String {
        format!(
            "correlation_degraded_total={} correlation_degraded_inflight_limit={} correlation_degraded_retired_limit={} correlation_missing_total={}",
            self.degraded_total.load(Ordering::Relaxed),
            self.degraded_inflight_limit.load(Ordering::Relaxed),
            self.degraded_retired_limit.load(Ordering::Relaxed),
            self.missing_total.load(Ordering::Relaxed),
        )
    }
}

/// One auxiliary record of a single forward, tagged with its correlation identity.
#[derive(Debug)]
enum RunEvent {
    /// Allowlisted transport facts, observed once per forward.
    Transport(ForwardMetadata),
    /// The interpreted request body of one forward.
    Request(ForwardId, Box<RequestObservation>),
    /// The interpreted response of one forward.
    Response(ForwardId, Box<ResponseObservation>),
    /// The classified transport failure of a forward that obtained no upstream response.
    TransportFailure(ForwardId, TransportFailure),
}

/// Synchronous, non-blocking bridge from the proxy sinks to the run recorder.
///
/// Both halves of a forward share one queue, so the transport status of a forward always
/// reaches the recorder before that forward's terminal semantic record.
#[derive(Debug)]
struct RecorderSink {
    sender: tokio::sync::mpsc::Sender<RunEvent>,
}

impl RecorderSink {
    fn offer(&self, event: RunEvent) -> Result<(), MetadataSinkError> {
        self.sender
            .try_send(event)
            .map_err(|_error| MetadataSinkError::rejected())
    }
}

impl MetadataSink for RecorderSink {
    fn try_record(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        self.offer(RunEvent::Transport(metadata))
    }
}

impl ProviderObservationSink for RecorderSink {
    fn try_record_request(
        &self,
        forward: ForwardId,
        observation: RequestObservation,
    ) -> Result<(), ObservationSinkError> {
        self.offer(RunEvent::Request(forward, Box::new(observation)))
    }

    fn try_record_response(
        &self,
        forward: ForwardId,
        observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        self.offer(RunEvent::Response(forward, Box::new(observation)))
    }

    fn try_record_transport_failure(
        &self,
        forward: ForwardId,
        failure: TransportFailure,
    ) -> Result<(), ObservationSinkError> {
        self.offer(RunEvent::TransportFailure(forward, failure))
    }
}

/// One semantic request observation awaiting its terminal response evidence.
#[derive(Debug)]
struct PendingObservation {
    request: RequestObservation,
    started_at: String,
}

/// Correlation state of one forward whose evidence is not yet settled.
///
/// Both semantic halves are buffered because the proxy delivers them from independent tasks:
/// whichever half completes the pair settles the forward, in either arrival order. A forward
/// that never obtained an upstream response is settled by its transport failure instead.
#[derive(Debug, Default)]
struct ForwardState {
    request: Option<PendingObservation>,
    response: Option<ResponseObservation>,
    status_code: Option<u16>,
    transport_failure: Option<TransportFailure>,
    transport: bool,
    settled: bool,
}

impl ForwardState {
    /// Takes the evidence observed for this forward so far.
    ///
    /// A response without its request half carries no logical request, so it is dropped in
    /// favour of whatever transport evidence the forward has, and the forward reports that its
    /// correlation is missing rather than presenting transport evidence as a whole exchange.
    fn evidence(&mut self) -> ForwardEvidence {
        let status_code = self.status_code;
        let response = self.response.take();
        let transport_failure = self.transport_failure.take();
        let orphaned_semantic =
            self.request.is_none() && (response.is_some() || transport_failure.is_some());
        ForwardEvidence {
            semantic: self.request.take().map(|pending| SemanticRecord {
                pending,
                response,
                status_code,
                transport_failure,
            }),
            orphaned_semantic,
            transport: self.transport,
        }
    }
}

/// Everything one settled forward contributes to durable storage.
#[derive(Debug)]
struct ForwardEvidence {
    semantic: Option<SemanticRecord>,
    /// Terminal semantic evidence was observed for a forward whose request half never arrived.
    orphaned_semantic: bool,
    transport: bool,
}

/// Semantic evidence of one forward, with the transport evidence its forwarding half observed.
#[derive(Debug)]
struct SemanticRecord {
    pending: PendingObservation,
    response: Option<ResponseObservation>,
    status_code: Option<u16>,
    transport_failure: Option<TransportFailure>,
}

/// Joins the transport and semantic records of every forward by correlation identity.
///
/// The proxy tags both halves of one forward with a [`ForwardId`], so each forward yields
/// exactly one inference operation: an observed Responses forward is owned by its semantic
/// record, which carries the upstream status the transport half observed, while every other
/// route keeps its transport-only record. Correlation state is bounded; when the bound is
/// reached the oldest identity is settled with the evidence it already has, and that identity
/// is retired so a half that arrives afterwards cannot open a second operation for a forward
/// already recorded, and every degradation of that bounded state is counted and reported.
/// Only an identity that entered correlation state is ever retired: an identity whose first
/// half is still being interpreted has been recorded nowhere, so it is admitted however far
/// behind the newest forward it has fallen.
#[derive(Debug)]
struct RunRecorder {
    config: Config,
    session_id: SessionId,
    parent_operation_id: OperationId,
    forwards: BTreeMap<ForwardId, ForwardState>,
    /// Identities dropped from correlation state that the watermark does not cover yet.
    ///
    /// Bounded by [`RETIRED_IDENTITIES`]; its contiguous prefix is folded into the watermark.
    retired: BTreeSet<ForwardId>,
    /// Watermark at or below which every identity has already been retired, once one exists.
    ///
    /// Identities are monotonic per proxy, so a contiguous run of retired identities collapses
    /// into one watermark: refusing everything at or below it refuses only forwards this
    /// recorder has already accounted for.
    retired_through: Option<ForwardId>,
    /// Correlation losses observed so far, shared with the run that reports them.
    counters: Arc<CorrelationCounters>,
}

impl RunRecorder {
    async fn run(
        mut self,
        mut events: tokio::sync::mpsc::Receiver<RunEvent>,
    ) -> Result<(), String> {
        while let Some(event) = events.recv().await {
            match event {
                RunEvent::Transport(metadata) => self.transport(metadata).await?,
                RunEvent::Request(forward, observation) => {
                    self.request(forward, *observation).await;
                }
                RunEvent::Response(forward, observation) => {
                    self.response(forward, *observation).await;
                }
                RunEvent::TransportFailure(forward, failure) => {
                    self.transport_failure(forward, failure).await;
                }
            }
        }
        // A forward still in flight when the run ends is recorded with what it has.
        while let Some((forward, mut state)) = self.forwards.pop_first() {
            self.retire(forward);
            if state.settled {
                continue;
            }
            let evidence = state.evidence();
            // The run ending is not a bound degradation: the forward keeps the evidence it has.
            self.record(evidence, CorrelationStatus::Correlated).await;
        }
        Ok(())
    }

    /// Records an unobserved route's forward, or joins the status onto a Responses forward.
    async fn transport(&mut self, metadata: ForwardMetadata) -> Result<(), String> {
        if !matches!(metadata.route, InboundRoute::Responses) {
            // Every route without semantic observation keeps its transport-only inference.
            return match self.record_forward().await? {
                ControlResponse::Ok { .. } => Ok(()),
                ControlResponse::Error { message } => Err(message),
            };
        }
        self.admit(metadata.forward).await;
        let Some(state) = self.forwards.get_mut(&metadata.forward) else {
            return Ok(());
        };
        if state.settled {
            return Ok(());
        }
        state.transport = true;
        state.status_code = metadata.status_code;
        Ok(())
    }

    /// Buffers the semantic request half, settling the forward once terminal evidence exists.
    async fn request(&mut self, forward: ForwardId, observation: RequestObservation) {
        // A clock failure is auxiliary: the forward keeps whatever other evidence arrives.
        let Ok(started_at) = current_timestamp() else {
            return;
        };
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        state.request = Some(PendingObservation {
            request: observation,
            started_at,
        });
        if state.response.is_none() && state.transport_failure.is_none() {
            return;
        }
        state.settled = true;
        let evidence = state.evidence();
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Buffers the terminal semantic response half, settling the forward once paired.
    ///
    /// The proxy delivers the request half from its own task, so it may still be in flight
    /// here; the forward then waits for it instead of losing its logical request.
    async fn response(&mut self, forward: ForwardId, observation: ResponseObservation) {
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        state.response = Some(observation);
        if state.request.is_none() {
            return;
        }
        state.settled = true;
        let evidence = state.evidence();
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Settles a forward that obtained no upstream response with its transport evidence.
    ///
    /// The forwarding half reports the failure from its own task, so the request half may still
    /// be in flight here; the forward then waits for it instead of losing its logical request.
    /// A failed forward is never left pending: its attempt is closed with the failure class.
    async fn transport_failure(&mut self, forward: ForwardId, failure: TransportFailure) {
        self.admit(forward).await;
        let Some(state) = self.forwards.get_mut(&forward) else {
            return;
        };
        if state.settled {
            return;
        }
        state.transport_failure = Some(failure);
        if state.request.is_none() {
            return;
        }
        state.settled = true;
        let evidence = state.evidence();
        self.record(evidence, CorrelationStatus::Correlated).await;
    }

    /// Reserves bounded correlation state for one forward identity.
    ///
    /// A settled identity is kept until eviction, and eviction retires it, so a half that
    /// arrives after its forward was recorded is refused instead of re-admitting a blank state
    /// that could produce a second inference operation for the same forward. An identity that
    /// never held correlation state is admitted whatever its ordinal: identities are allocated
    /// before a body is read, so a forward whose bounded parse is expensive reaches the recorder
    /// after cheaper, newer forwards have already evicted state, and refusing it would leave the
    /// forward with no record at all.
    async fn admit(&mut self, forward: ForwardId) {
        if self.forwards.contains_key(&forward) {
            return;
        }
        if self.is_retired(forward) {
            // This forward was already recorded with the evidence it had, so this half can no
            // longer be joined to it and re-admitting it would record the forward twice. The
            // evidence is lost either way; what must not be lost is that it was.
            self.degrade(CorrelationDegradation::RetiredLimit).await;
            return;
        }
        while self.forwards.len() >= IN_FLIGHT_FORWARDS {
            // Identities are monotonic, so the first key is deterministically the oldest.
            let Some((oldest, mut state)) = self.forwards.pop_first() else {
                break;
            };
            self.retire(oldest);
            if state.settled {
                continue;
            }
            let evidence = state.evidence();
            // The bound, not the exchange, ended this forward's correlation: it is recorded
            // with the evidence it has and marked degraded, never as a whole exchange.
            self.record(
                evidence,
                CorrelationStatus::Degraded(CorrelationDegradation::InFlightLimit),
            )
            .await;
        }
        let _admitted = self.forwards.insert(forward, ForwardState::default());
    }

    /// Reports whether this identity already held correlation state and lost it.
    fn is_retired(&self, forward: ForwardId) -> bool {
        self.retired_through
            .is_some_and(|watermark| forward <= watermark)
            || self.retired.contains(&forward)
    }

    /// Retires one identity that correlation state no longer holds.
    fn retire(&mut self, forward: ForwardId) {
        if self.is_retired(forward) {
            return;
        }
        let _retired = self.retired.insert(forward);
        // Identities start at zero and are monotonic, so a contiguous run of retirements is
        // exactly what the watermark states and costs nothing to keep past the fold.
        while let Some(smallest) = self.retired.first().copied() {
            let next = self
                .retired_through
                .map_or(0, |watermark| watermark.get().saturating_add(1));
            if smallest.get() != next {
                break;
            }
            let _folded = self.retired.pop_first();
            self.retired_through = Some(smallest);
        }
        // An identity the proxy never reported an event for — an oversized request body is
        // rejected before any observation — blocks that fold for the rest of the run, so the
        // retirements stranded above it are bounded rather than kept forever.
        while self.retired.len() > RETIRED_IDENTITIES {
            let _forgotten = self.retired.pop_first();
        }
    }

    /// Records the single inference operation one settled forward is entitled to.
    ///
    /// Semantic evidence owns that operation whenever the record reaches the daemon, including
    /// when the daemon rejects it: the daemon owns the outcome either way, so the same forward
    /// never gets a second operation. A record the CLI could not deliver at all — a dropped or
    /// oversized observation — falls back to the transport record, so the forward still leaves
    /// evidence behind.
    async fn record(&self, evidence: ForwardEvidence, correlation: CorrelationStatus) {
        let ForwardEvidence {
            semantic,
            orphaned_semantic,
            transport,
        } = evidence;
        if let CorrelationStatus::Degraded(reason) = correlation {
            self.counters.degraded(reason);
        }
        // The record carries its own correlation status, so the daemon commits the degradation
        // event in the very transaction that persists the evidence.
        let recorded = match semantic {
            Some(semantic) => self.record_observation(semantic, correlation).await,
            None => false,
        };
        if !recorded {
            // No record reached the daemon to carry the degradation, so it is reported alone.
            if let CorrelationStatus::Degraded(reason) = correlation {
                self.report_degradation(reason).await;
            }
            if transport {
                // Fallback transport evidence is auxiliary: its failure never fails the run.
                drop(self.record_forward().await);
            }
        }
        if orphaned_semantic {
            // Terminal evidence without a request half is a degradation of its own, whatever
            // else happened to this forward: no logical request can be recorded for it.
            self.degrade(CorrelationDegradation::MissingRequestHalf)
                .await;
        }
    }

    /// Counts one degradation no forward record can carry and makes it observable.
    async fn degrade(&self, reason: CorrelationDegradation) {
        self.counters.degraded(reason);
        self.report_degradation(reason).await;
    }

    /// Asks the daemon to commit the canonical event of one correlation degradation.
    ///
    /// The message carries the reason, the session, and the moment only: a degradation is
    /// missing evidence, so it may not carry the forward it degraded.
    async fn report_degradation(&self, reason: CorrelationDegradation) {
        let Ok(observed_at) = current_timestamp() else {
            return;
        };
        // Observing the loss is itself auxiliary: a rejected event never fails the run.
        drop(
            control(
                &self.config,
                ControlRequest::RecordCorrelationDegradation {
                    session_id: self.session_id,
                    reason,
                    observed_at,
                },
            )
            .await,
        );
    }

    /// Sends one semantic record, reporting whether the daemon owns the resulting operation.
    async fn record_observation(
        &self,
        semantic: SemanticRecord,
        correlation: CorrelationStatus,
    ) -> bool {
        let Ok(ended_at) = current_timestamp() else {
            return false;
        };
        let Some(observation) = observation_record(semantic, ended_at, correlation) else {
            return false;
        };
        control(
            &self.config,
            ControlRequest::RecordProviderObservation {
                session_id: self.session_id,
                parent_operation_id: self.parent_operation_id,
                observation: Box::new(observation),
            },
        )
        .await
        .is_ok()
    }

    /// Records one transport-only inference operation beneath the agent root.
    async fn record_forward(&self) -> Result<ControlResponse, String> {
        control(
            &self.config,
            ControlRequest::RecordForward {
                session_id: self.session_id,
                parent_operation_id: self.parent_operation_id,
                observed_at: current_timestamp()?,
            },
        )
        .await
    }
}

fn observation_record(
    semantic: SemanticRecord,
    ended_at: String,
    correlation: CorrelationStatus,
) -> Option<ObservationRecord> {
    let SemanticRecord {
        pending,
        response,
        status_code,
        transport_failure,
    } = semantic;
    // The parser always measures its bounded input; an unmeasured body is never invented.
    let request_bytes = pending.request.request_bytes?;
    let streaming = pending.request.stream;
    // An unobserved or unrepresentable status stays unknown rather than fabricated.
    let status_code = status_code.and_then(|value| HttpStatusCode::new(value).ok());
    // An errored upstream status is terminal evidence by itself: the exchange is over even when
    // nothing interpreted the body, which is what an upstream answering a media type the
    // observer does not read leaves behind. Closing the attempt here keeps the timestamp a real
    // local measurement instead of one the daemon would have to invent, and keeps the daemon
    // from persisting an errored attempt whose `ended_at` is still NULL.
    let errored_upstream = status_code.is_some_and(|code| !(200..300).contains(&code.get()));
    let terminal = response.is_some() || transport_failure.is_some() || errored_upstream;
    let mut record = ObservationRecord::new(request_bytes, pending.request, pending.started_at)
        .with_correlation_status(correlation);
    if let Some(streaming) = streaming {
        record = record.with_streaming(streaming);
    }
    if let Some(status_code) = status_code {
        record = record.with_status_code(status_code);
    }
    if let Some(response) = response {
        let outcome = observation_outcome(response.response_state);
        record = record.with_response(response).with_outcome(outcome);
    }
    // The failure class is content-free evidence that this forward reached no response at all,
    // so it overrides any semantic outcome and closes the attempt.
    if let Some(failure) = transport_failure {
        record = record.with_transport_error(failure.label());
    }
    if terminal {
        record = record.with_ended_at(ended_at);
    }
    Some(record)
}

const fn observation_outcome(state: ProviderResponseState) -> ProviderObservationOutcome {
    match state {
        ProviderResponseState::Completed => ProviderObservationOutcome::Completed,
        ProviderResponseState::Incomplete => ProviderObservationOutcome::Incomplete,
        ProviderResponseState::Failed => ProviderObservationOutcome::Failed,
        ProviderResponseState::Cancelled => ProviderObservationOutcome::Cancelled,
        ProviderResponseState::Disconnected => ProviderObservationOutcome::Disconnected,
        _ => ProviderObservationOutcome::InProgress,
    }
}

/// Session identities a recorder attributes one run's evidence to.
#[derive(Debug)]
struct RunRecording {
    config: Config,
    session_id: SessionId,
    parent_operation_id: OperationId,
}

/// The recorder of one run: its proxy, its worker, and the counters it publishes.
struct SpawnedRecorder {
    proxy: TransparentProxy,
    task: tokio::task::JoinHandle<Result<(), String>>,
    /// Correlation accounting the run reports, readable whether or not the worker finished.
    counters: Arc<CorrelationCounters>,
}

/// Installs both auxiliary sinks and starts the worker that drains their shared queue.
fn spawn_recorder(recording: RunRecording, proxy: TransparentProxy) -> SpawnedRecorder {
    let RunRecording {
        config,
        session_id,
        parent_operation_id,
    } = recording;
    let (sender, receiver) = tokio::sync::mpsc::channel(RECORDER_QUEUE_ITEMS);
    let counters = Arc::new(CorrelationCounters::default());
    let task = tokio::spawn(
        RunRecorder {
            config,
            session_id,
            parent_operation_id,
            forwards: BTreeMap::new(),
            retired: BTreeSet::new(),
            retired_through: None,
            counters: Arc::clone(&counters),
        }
        .run(receiver),
    );
    let proxy = proxy
        .with_metadata_sink(Arc::new(RecorderSink {
            sender: sender.clone(),
        }))
        .with_observation_sink(Arc::new(RecorderSink { sender }));
    SpawnedRecorder {
        proxy,
        task,
        counters,
    }
}

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
    Proxy,
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
    let mut bytes = [0_u8; 32];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
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

async fn run_agent(config: &Config, agent: String, args: Vec<String>) -> Result<(), String> {
    if !daemon_running(config).await? {
        return Err("daemon is not running; run `tracepress daemon start` first".to_owned());
    }
    let upstream = std::env::var("TRACEPRESS_UPSTREAM")
        .map_err(|_| "TRACEPRESS_UPSTREAM is required by `tracepress run`")?;
    let endpoint = ProviderEndpoint::new(&upstream).map_err(|error| error.to_string())?;
    let proxy = TransparentProxy::new(ProxyConfig::new(
        endpoint,
        MaxRequestBodyBytes::new(8 * 1024 * 1024).map_err(|e| e.to_string())?,
        MaxResponseBodyBytes::new(32 * 1024 * 1024).map_err(|e| e.to_string())?,
    ))
    .map_err(|error| error.to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let proxy_address = listener.local_addr().map_err(|error| error.to_string())?;
    let started_at = current_timestamp()?;
    let response = control(config, ControlRequest::StartSession { started_at }).await?;
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
    };
    let SpawnedRecorder {
        proxy,
        task: mut recorder_task,
        counters,
    } = spawn_recorder(
        RunRecording {
            config: config.clone(),
            session_id: session.session_id,
            parent_operation_id,
        },
        proxy,
    );
    let proxy_task = tokio::spawn(async move { serve(listener, proxy.router()).await });
    let base_url = format!("http://{proxy_address}/v1");
    let status_result = Command::new(&agent)
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
    proxy_task.abort();
    let _proxy_result = proxy_task.await;
    // Recording is auxiliary: draining is bounded so it can never hold up the run.
    let recorded = match tokio::time::timeout(
        Duration::from_secs(RECORDER_DRAIN_SECONDS),
        &mut recorder_task,
    )
    .await
    {
        Ok(joined) => joined.map_err(|error| format!("recording worker failed: {error}"))?,
        Err(_elapsed) => {
            recorder_task.abort();
            Ok(())
        }
    };
    // Correlation degradation is reported for every run, before anything else can fail: a run
    // whose bounded state lost evidence must never look like a run that lost none.
    println!("{}", counters.report());
    let ended_at = current_timestamp()?;
    let finalization = control(
        config,
        ControlRequest::FinishSession {
            session_id: session.session_id,
            ended_at,
        },
    )
    .await;
    let status = status_result.map_err(|error| format!("cannot launch agent {agent}: {error}"))?;
    recorded.map_err(|error| format!("forward recording failed: {error}"))?;
    match finalization? {
        ControlResponse::Ok { .. } => {}
        ControlResponse::Error { message } => {
            return Err(format!(
                "agent finished but session finalization failed: {message}"
            ));
        }
    }
    match status.code() {
        Some(code) if code != 0 => Err(format!("agent exited with status {code}")),
        Some(_) => Ok(()),
        None => Err("agent terminated by signal".to_owned()),
    }
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
    let upstream = std::env::var("TRACEPRESS_UPSTREAM")
        .map_err(|_| "TRACEPRESS_UPSTREAM must be set to /v1/chat/completions")?;
    let endpoint = ProviderEndpoint::new(&upstream).map_err(|error| error.to_string())?;
    let request_limit = MaxRequestBodyBytes::new(8 * 1024 * 1024).map_err(|e| e.to_string())?;
    let response_limit = MaxResponseBodyBytes::new(32 * 1024 * 1024).map_err(|e| e.to_string())?;
    let proxy = TransparentProxy::new(ProxyConfig::new(endpoint, request_limit, response_limit))
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
    serve(listener, proxy.router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| error.to_string())
}

async fn doctor(config: &Config) -> Result<(), String> {
    config.ensure_root()?;
    let _credential = credential(config)?;
    println!("state: {}", config.root.display());
    println!(
        "daemon: {}",
        if daemon_running(config).await? {
            "running"
        } else {
            "stopped"
        }
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let cli = Cli::parse();
    let config = Config::load()?;
    match cli.command {
        CommandKind::Init => init(&config).await,
        CommandKind::Doctor => doctor(&config).await,
        CommandKind::Daemon {
            command: DaemonCommand::Start,
        } => daemon_start(&config).await,
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
    }
}
