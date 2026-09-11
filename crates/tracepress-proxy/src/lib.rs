#![allow(
    clippy::multiple_crate_versions,
    reason = "the HTTP client and TLS stack currently require distinct transitive platform crates"
)]

//! Transparent OpenAI-compatible HTTP forwarding for Tracepress Phase 1.

pub(crate) mod decoding;

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, StatusCode, Uri};
use axum::response::Response;
use axum::routing::post;
use decoding::{
    AnalysisDecoder, BoundedAnalysisDecoder, DecodeLimits, DecodeResult, WireBody,
    parse_content_encoding_header,
};
use futures_util::{StreamExt as _, stream};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio::sync::{Notify, Semaphore};
pub use tracepress_context::ContextAnalysisDropReason;
use tracepress_context::{
    ContextAnalysisLimits, ContextAnalysisLimitsError, ContextAnalysisResult, ContextDigest,
    analyze_responses,
};
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes, ResourceLimits};
pub use tracepress_provider::ContentEncoding;
use tracepress_provider::{
    AnalysisDecodeStatus, MAX_RETAINED_USAGE_BYTES, ObservationInput, ObservationLimitValues,
    ObservationLimits, ObservationStatus, OpenAiResponsesV1Observer, ProviderEndpoint,
    ProviderObserver, ProviderTransport, RequestObservation, ResponseObservation,
    StreamingObserver,
};

/// Gives a newly accepted N+1 forward priority over detached analysis from N.
///
/// The grace period is deliberately longer than one scheduler tick so an immediately following
/// request can publish its active-forward guard before analysis acquires CPU.
const CONTEXT_ANALYSIS_QUIESCENCE_WINDOW: Duration = Duration::from_millis(10);
const OBSERVATION_QUEUE_CAPACITY: usize = 32;

/// Hard cap on the resource-derived context-analysis execution ceiling.
///
/// Each analysis retains a refcounted request prefix and up to the analyzer's bounded block
/// drafts, so allowing every runtime worker to analyze at once would turn burst width into
/// memory pressure. The effective value is the smaller of this cap, available parallelism, and
/// the configured CPU-work budget. The deferred queue currently uses one FIFO worker per
/// proxy/session; the derived value remains a diagnostic/configuration bound.
const CONTEXT_ANALYSIS_CONCURRENCY_HARD_CAP: usize = 2;
/// Bounds concurrent request decoders independently of the Phase 3 analysis slots.
const OBSERVATION_DECODE_CONCURRENCY_HARD_CAP: usize = 8;
/// Maximum number of raw wire bodies retained for deferred context analysis.
const DEFERRED_ANALYSIS_MAX_ITEMS: usize = 32;
/// Maximum compressed/raw wire bytes retained for deferred context analysis.
const DEFERRED_ANALYSIS_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Correlation identity of one forwarded provider request.
///
/// The transport metadata record and every semantic observation of the same forward carry this
/// scalar, so a consumer joins the two streams by identity instead of arrival order. Values are
/// monotonic per proxy and contain no request content.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ForwardId(u64);

impl ForwardId {
    /// Returns the monotonic ordinal this forward was accepted at.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}
/// Keeps one accepted forward counted until its response body settles or the forward fails.
struct ActiveForwardGuard {
    active: Arc<AtomicU64>,
    notify: Arc<Notify>,
}

impl ActiveForwardGuard {
    fn acquire(active: &Arc<AtomicU64>, notify: &Arc<Notify>) -> Self {
        let _ = active.fetch_add(1, Ordering::AcqRel);
        Self {
            active: Arc::clone(active),
            notify: Arc::clone(notify),
        }
    }
}

impl Drop for ActiveForwardGuard {
    fn drop(&mut self) {
        let previous = self.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "active forward counter underflow");
        self.notify.notify_waiters();
    }
}
#[derive(Clone, Debug)]
struct BackgroundTracker {
    pending: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

struct BackgroundGuard {
    tracker: BackgroundTracker,
}

impl BackgroundTracker {
    fn new() -> Self {
        Self {
            pending: Arc::new(AtomicUsize::new(0)),
            notify: Arc::new(Notify::new()),
        }
    }

    fn guard(&self) -> BackgroundGuard {
        let _ = self.pending.fetch_add(1, Ordering::AcqRel);
        BackgroundGuard {
            tracker: self.clone(),
        }
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.pending.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

impl Drop for BackgroundGuard {
    fn drop(&mut self) {
        let previous = self.tracker.pending.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "background task counter underflow");
        self.tracker.notify.notify_waiters();
    }
}

/// Admission accounting for deferred analysis bodies.
///
/// The budget covers queued and currently executing jobs together. This keeps the raw wire body
/// retained by an analysis job bounded by both item count and bytes, without making forwarding
/// wait for an analysis permit.
#[derive(Debug)]
struct DeferredAnalysisBudget {
    state: Mutex<DeferredAnalysisBudgetState>,
    max_items: usize,
    max_bytes: u64,
}

#[derive(Debug, Default)]
struct DeferredAnalysisBudgetState {
    items: usize,
    bytes: u64,
}

#[derive(Debug)]
struct DeferredAnalysisPermit {
    budget: Arc<DeferredAnalysisBudget>,
    bytes: u64,
    queue: std::sync::Weak<DeferredAnalysisQueue>,
    forward: Option<ForwardId>,
}

/// Bounded deferred-analysis counters exposed for diagnostics and benchmark reports.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct DeferredAnalysisMetrics {
    /// Current jobs retained, including one currently executing.
    pub queue_items: u64,
    /// Current retained raw wire bytes.
    pub queue_bytes: u64,
    /// Highest observed retained job count.
    pub high_water_items: u64,
    /// Highest observed retained raw wire bytes.
    pub high_water_bytes: u64,
    /// Number of requests admitted to deferred analysis.
    pub deferred_total: u64,
    /// Number of admitted jobs whose analysis was handed to the sink.
    pub processed_deferred_total: u64,
    /// Number of requests rejected by the item or byte admission bound.
    pub backlog_capacity_drops: u64,
    /// Sum of time jobs spent waiting before the worker picked them up.
    pub analysis_wait_us: u64,
}

impl DeferredAnalysisBudget {
    fn new(max_items: usize, max_bytes: u64) -> Self {
        Self {
            state: Mutex::new(DeferredAnalysisBudgetState::default()),
            max_items: max_items.max(1),
            max_bytes,
        }
    }

    fn try_reserve(
        self: &Arc<Self>,
        bytes: u64,
    ) -> Result<DeferredAnalysisPermit, ContextAnalysisDropReason> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(next_items) = state.items.checked_add(1) else {
            return Err(ContextAnalysisDropReason::DeferredBacklogCapacity);
        };
        let Some(next_bytes) = state.bytes.checked_add(bytes) else {
            return Err(ContextAnalysisDropReason::DeferredBacklogCapacity);
        };
        if next_items > self.max_items || next_bytes > self.max_bytes {
            return Err(ContextAnalysisDropReason::DeferredBacklogCapacity);
        }
        state.items = next_items;
        state.bytes = next_bytes;
        drop(state);
        Ok(DeferredAnalysisPermit {
            budget: Arc::clone(self),
            bytes,
            queue: std::sync::Weak::new(),
            forward: None,
        })
    }

    fn release(&self, bytes: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(
            state.items > 0,
            "deferred analysis item accounting underflow"
        );
        debug_assert!(
            state.bytes >= bytes,
            "deferred analysis byte accounting underflow"
        );
        state.items = state.items.saturating_sub(1);
        state.bytes = state.bytes.saturating_sub(bytes);
    }

    fn usage(&self) -> (usize, u64) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.items, state.bytes)
    }
}

impl Drop for DeferredAnalysisPermit {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
        if let (Some(queue), Some(forward)) = (self.queue.upgrade(), self.forward) {
            queue.cancel_admission(forward);
        }
    }
}

/// Schedules bounded auxiliary work while keeping the proxy's shutdown barrier live.
#[derive(Clone, Debug)]
pub struct BackgroundTaskSpawner {
    tracker: BackgroundTracker,
}

impl BackgroundTaskSpawner {
    /// Runs one blocking auxiliary task without holding up the forwarding task.
    ///
    /// The tracker guard is moved into the task before it is submitted, so a shutdown drain
    /// cannot finish until the task has either sent its event or exited. Returns `false` only
    /// when called outside a Tokio runtime.
    pub fn spawn_blocking<F>(&self, task: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let guard = self.tracker.guard();
        drop(handle.spawn_blocking(move || {
            let _guard = guard;
            task();
        }));
        true
    }
}

/// Selects whether the proxy performs detached Phase 3 request analysis.
///
/// `Off` leaves Phase 2 provider request/response observation enabled while skipping every
/// context-analysis task and handoff. `Shadow` performs bounded analysis after the response body
/// has settled and never changes forwarded bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContextAnalysisMode {
    /// Do not run or persist Phase 3 context analysis.
    Off,
    /// Run bounded Phase 3 analysis as a detached shadow side effect.
    Shadow,
}

/// Inbound route one request was accepted on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InboundRoute {
    /// `/v1/chat/completions`, forwarded without semantic observation.
    ChatCompletions,
    /// `/v1/responses`, forwarded with semantic observation.
    Responses,
    /// `/v1/responses/compact`, forwarded as provider-managed compaction transport only.
    ResponsesCompact,
}

impl InboundRoute {
    const fn path(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
            Self::ResponsesCompact => "/v1/responses/compact",
        }
    }
}

/// Allowlisted transport facts emitted without headers or body content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ForwardMetadata {
    /// Correlation identity shared with this forward's semantic observations.
    pub forward: ForwardId,
    /// Inbound route this request was accepted on.
    pub route: InboundRoute,
    /// Exact accepted request body length.
    pub request_bytes: u64,
    /// Upstream status when transport reached response headers.
    pub status_code: Option<u16>,
    /// Response length remains unknown until the stream completes.
    pub response_bytes: Option<u64>,
}

/// Failure from a non-blocking auxiliary metadata sink.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("metadata sink rejected the record")]
#[non_exhaustive]
pub struct MetadataSinkError;

impl MetadataSinkError {
    /// Creates a sink-local rejection.
    #[must_use]
    pub const fn rejected() -> Self {
        Self
    }
}

/// Compatibility name for the failure returned by semantic observation sinks.
pub type ObservationSinkError = MetadataSinkError;

/// Synchronous, non-blocking boundary for optional transport metadata.
pub trait MetadataSink: Send + Sync + 'static {
    /// Attempts to accept allowlisted metadata without affecting forwarding.
    ///
    /// # Errors
    /// Returns a sink-local failure that the proxy deliberately ignores.
    fn try_record(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError>;
}

/// Non-sensitive classification of a forward that never obtained an upstream response.
///
/// The label states why transport failed and carries no headers, endpoint URL or body bytes, so a
/// durable consumer can separate a failed attempt from one still in flight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransportFailure {
    /// The upstream connection could not be established.
    Connect,
    /// The upstream did not answer within the client's timeout.
    Timeout,
    /// The upstream request could not be built or sent.
    Request,
    /// The configured endpoint could not be rewritten into a request URI.
    Endpoint,
    /// Transport failed for a reason the proxy does not classify further.
    Other,
}

/// Terminal transport outcome of a provider-managed compaction exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompactionOutcome {
    /// The upstream response body settled with a successful HTTP status.
    Completed,
    /// The upstream returned an error status or failed before response headers.
    Failed,
    /// The response ended before its body settled successfully.
    Incomplete,
    /// The downstream cancelled the response.
    Cancelled,
    /// The upstream disconnected while its body was being forwarded.
    Disconnected,
}

/// Bounded, content-free evidence of one `/responses/compact` exchange.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CompactionObservation {
    /// Correlation identity allocated for the accepted forward.
    pub forward: ForwardId,
    /// Exact request body bytes accepted and forwarded.
    pub request_bytes: u64,
    /// SHA-256 of the exact request body sent upstream.
    pub wire_sha256: [u8; 32],
    /// Content encoding retained on the wire.
    pub content_encoding: ContentEncoding,
    /// Provider transport profile used for the exchange.
    pub transport: ProviderTransport,
    /// Version of the endpoint profile used for the exchange.
    pub endpoint_profile_version: u32,
    /// Upstream response status, when response headers were received.
    pub status_code: Option<u16>,
    /// Exact response bytes forwarded to the downstream consumer.
    pub response_bytes: u64,
    /// Local monotonic duration from upstream dispatch to body settlement.
    pub duration_us: Option<u64>,
    /// Terminal transport outcome.
    pub outcome: CompactionOutcome,
    /// Content-free pre-response failure classification, when applicable.
    pub transport_failure: Option<TransportFailure>,
}

impl TransportFailure {
    /// Returns the stable, content-free label of this failure class.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Timeout => "timeout",
            Self::Request => "request",
            Self::Endpoint => "endpoint",
            Self::Other => "other",
        }
    }

    /// Classifies a client failure raised before any upstream response existed.
    fn classify(error: &reqwest::Error) -> Self {
        if error.is_connect() {
            Self::Connect
        } else if error.is_timeout() {
            Self::Timeout
        } else if error.is_builder() || error.is_request() {
            Self::Request
        } else {
            Self::Other
        }
    }
}

/// Result of the context-analysis handoff for one forwarded request.
///
/// A drop is explicit: the sink receives no fabricated digest, hash, status, or block list.
#[derive(Debug)]
#[non_exhaustive]
pub enum ContextAnalysisOutcome {
    /// The analyzer produced the bounded result for the analysis bytes; forwarding retained the
    /// original wire representation.
    Analyzed(ContextAnalysisResult),
    /// The analyzer was not run or its result could not be handed to the sink.
    Dropped(ContextAnalysisDropReason),
}

/// A detached Phase 3 context-analysis outcome tagged with its forwarding identity.
///
/// The provider recorder accepts this independently from the request observation so provider
/// persistence can settle before analysis finishes.
#[derive(Debug)]
#[non_exhaustive]
pub struct ContextAnalysisObservation {
    /// Correlation identity shared with this forward's request and response evidence.
    pub forward: ForwardId,
    /// Analysis result or explicit reason it was dropped.
    pub outcome: ContextAnalysisOutcome,
}

/// A parsed request observation and its detached context-analysis outcome for one forward.
///
/// Phase 2 request metadata may be handed to the sink before upstream headers are available.
/// The optional outcome is populated only when a caller already has the detached Phase 3 result;
/// the proxy's normal path sends that outcome through [`ProviderObservationSink::try_record_context_analysis`]
/// after dispatch.
#[derive(Debug)]
#[non_exhaustive]
pub struct RequestContextObservation {
    /// Correlation identity shared with this forward's metadata and response evidence.
    pub forward: ForwardId,
    /// Parsed provider request metadata.
    pub observation: RequestObservation,
    /// Detached shadow context analysis outcome, when it is already available.
    pub context: Option<ContextAnalysisOutcome>,
}
/// Synchronous, non-blocking boundary for Responses v1 semantic observations.
pub trait ProviderObservationSink: Send + Sync + 'static {
    /// Attempts to accept a parsed request observation for one forward.
    ///
    /// A detached Phase 3 result, when available, is handed to
    /// [`Self::try_record_context_analysis`] independently.
    /// # Errors
    /// Returns a sink-local failure that the proxy deliberately ignores.
    fn try_record_request_context(
        &self,
        observation: RequestContextObservation,
    ) -> Result<(), ObservationSinkError>;
    /// Attempts to accept the detached Phase 3 outcome for one already parsed request.
    ///
    /// This separate handoff lets Phase 2 provider persistence happen before analysis finishes
    /// while ensuring no Phase 3 analysis starts before dispatch.
    ///
    /// # Errors
    /// Returns a sink-local failure that the proxy deliberately ignores.
    fn try_record_context_analysis(
        &self,
        observation: ContextAnalysisObservation,
    ) -> Result<(), ObservationSinkError>;
    /// Attempts to accept a parsed response observation of one forward.
    ///
    /// # Errors
    /// Returns a sink-local failure that the proxy deliberately ignores.
    fn try_record_response(
        &self,
        forward: ForwardId,
        observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError>;

    /// Attempts to accept the transport failure of a forward that reached no upstream response.
    ///
    /// Called on the observed route when forwarding itself failed, so the attempt can be closed
    /// with evidence instead of staying indistinguishable from one still in flight.
    ///
    /// # Errors
    /// Returns a sink-local failure that the proxy deliberately ignores.
    fn try_record_transport_failure(
        &self,
        forward: ForwardId,
        failure: TransportFailure,
    ) -> Result<(), ObservationSinkError>;

    /// Attempts to accept bounded transport evidence for a compaction request.
    ///
    /// Compaction is intentionally not passed through the Responses semantic parser. The
    /// default keeps existing sinks source-compatible while callers that persist provider
    /// evidence can opt into the dedicated request kind.
    ///
    /// # Errors
    /// Returns a sink-local failure when the sink cannot accept the bounded observation.
    fn try_record_compaction(
        &self,
        _observation: CompactionObservation,
    ) -> Result<(), ObservationSinkError> {
        Ok(())
    }
}

#[derive(Debug)]
struct NoopMetadataSink;

impl MetadataSink for NoopMetadataSink {
    fn try_record(&self, _metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        Ok(())
    }
}

#[derive(Debug)]
struct NoopProviderObservationSink;

impl ProviderObservationSink for NoopProviderObservationSink {
    fn try_record_request_context(
        &self,
        _observation: RequestContextObservation,
    ) -> Result<(), ObservationSinkError> {
        Ok(())
    }

    fn try_record_context_analysis(
        &self,
        _observation: ContextAnalysisObservation,
    ) -> Result<(), ObservationSinkError> {
        Ok(())
    }

    fn try_record_response(
        &self,
        _forward: ForwardId,
        _observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        Ok(())
    }

    fn try_record_transport_failure(
        &self,
        _forward: ForwardId,
        _failure: TransportFailure,
    ) -> Result<(), ObservationSinkError> {
        Ok(())
    }
}

/// Failure to validate a transparent proxy configuration.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProxyConfigError {
    /// Derived context-analysis bounds cannot be represented on this platform.
    #[error("context analysis limits are invalid")]
    ContextAnalysisLimits(#[source] ContextAnalysisLimitsError),
}

/// Validated transparent proxy configuration.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// Full upstream endpoint used as the authority and query base for both routes.
    pub upstream: ProviderEndpoint,
    /// Complete validated process resource limits shared by forwarding and analysis.
    pub resource_limits: ResourceLimits,
    /// Explicit Phase 3 context-analysis behavior.
    pub context_analysis_mode: ContextAnalysisMode,
    /// Maximum accepted request body size.
    pub max_request_body_bytes: MaxRequestBodyBytes,
    /// Maximum streamed response body size.
    pub max_response_body_bytes: MaxResponseBodyBytes,
    /// Bounded shadow context limits derived from `resource_limits`.
    pub context_analysis_limits: ContextAnalysisLimits,
    /// Resource-derived context-analysis execution ceiling. The deferred queue currently uses
    /// one FIFO worker per proxy/session, so this remains a diagnostic/configuration bound.
    pub context_analysis_concurrency: NonZeroUsize,
}

impl ProxyConfig {
    /// Creates a transparent proxy configuration from the complete resource budget.
    ///
    /// # Errors
    /// Returns [`ProxyConfigError::ContextAnalysisLimits`] when the derived analysis bounds
    /// cannot be represented on this platform.
    pub fn new(
        upstream: ProviderEndpoint,
        resource_limits: ResourceLimits,
        context_analysis_mode: ContextAnalysisMode,
    ) -> Result<Self, ProxyConfigError> {
        let context_analysis_limits = ContextAnalysisLimits::from_resource_limits(&resource_limits)
            .map_err(ProxyConfigError::ContextAnalysisLimits)?;
        let context_analysis_concurrency =
            derive_context_analysis_concurrency(&resource_limits, context_analysis_limits);
        Ok(Self {
            upstream,
            max_request_body_bytes: resource_limits.max_request_body_bytes,
            max_response_body_bytes: resource_limits.max_response_body_bytes,
            resource_limits,
            context_analysis_mode,
            context_analysis_limits,
            context_analysis_concurrency,
        })
    }
}

fn derive_context_analysis_concurrency(
    resource_limits: &ResourceLimits,
    context_analysis_limits: ContextAnalysisLimits,
) -> NonZeroUsize {
    let available_parallelism = std::thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .max(1);
    derive_context_analysis_concurrency_with_parallelism(
        resource_limits,
        context_analysis_limits,
        available_parallelism,
    )
}

fn derive_context_analysis_concurrency_with_parallelism(
    resource_limits: &ResourceLimits,
    context_analysis_limits: ContextAnalysisLimits,
    available_parallelism: usize,
) -> NonZeroUsize {
    let available_parallelism = available_parallelism.max(1);
    let configured_capacity = resource_limits
        .max_cpu_work_units
        .get()
        .checked_div(context_analysis_limits.max_analysis_work_units.get())
        .unwrap_or(1)
        .max(1);
    let configured_capacity = usize::try_from(configured_capacity).unwrap_or(usize::MAX);
    let capacity = available_parallelism
        .min(configured_capacity)
        .clamp(1, CONTEXT_ANALYSIS_CONCURRENCY_HARD_CAP);
    NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::MIN)
}

/// One raw-body analysis retained until the single session-local worker processes it.
struct DeferredAnalysisJob {
    forward: ForwardId,
    wire_body: WireBody,
    content_encoding: ContentEncoding,
    decode_limits: DecodeLimits,
    context_limits: ContextAnalysisLimits,
    wire_content_hash: ContextDigest,
    sink: Arc<dyn ProviderObservationSink>,
    admission: DeferredAnalysisPermit,
    enqueued_at: Instant,
}

struct DeferredAnalysisQueue {
    budget: Arc<DeferredAnalysisBudget>,
    state: Mutex<DeferredAnalysisQueueState>,
    notify: Notify,
    background: BackgroundTracker,
    active_forwards: Arc<AtomicU64>,
    active_notify: Arc<Notify>,
    high_water_items: AtomicUsize,
    high_water_bytes: AtomicU64,
    deferred_total: AtomicU64,
    processed_deferred_total: AtomicU64,
    backlog_capacity_drops: AtomicU64,
    analysis_wait_us: AtomicU64,
}

#[derive(Default)]
struct DeferredAnalysisQueueState {
    admitted: BTreeSet<ForwardId>,
    jobs: BTreeMap<ForwardId, DeferredAnalysisJob>,
    worker_running: bool,
}

enum DeferredAnalysisNext {
    Ready(Box<DeferredAnalysisJob>),
    Waiting,
    Empty,
}

impl DeferredAnalysisQueue {
    fn new(
        background: BackgroundTracker,
        active_forwards: Arc<AtomicU64>,
        active_notify: Arc<Notify>,
    ) -> Arc<Self> {
        Arc::new(Self {
            budget: Arc::new(DeferredAnalysisBudget::new(
                DEFERRED_ANALYSIS_MAX_ITEMS,
                DEFERRED_ANALYSIS_MAX_BYTES,
            )),
            state: Mutex::new(DeferredAnalysisQueueState::default()),
            notify: Notify::new(),
            background,
            active_forwards,
            active_notify,
            high_water_items: AtomicUsize::new(0),
            high_water_bytes: AtomicU64::new(0),
            deferred_total: AtomicU64::new(0),
            processed_deferred_total: AtomicU64::new(0),
            backlog_capacity_drops: AtomicU64::new(0),
            analysis_wait_us: AtomicU64::new(0),
        })
    }

    fn try_admit(
        self: &Arc<Self>,
        forward: ForwardId,
        wire_bytes: u64,
    ) -> Result<DeferredAnalysisPermit, ContextAnalysisDropReason> {
        let result = self.budget.try_reserve(wire_bytes);
        match result {
            Ok(mut permit) => {
                permit.queue = Arc::downgrade(self);
                permit.forward = Some(forward);
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let _inserted = state.admitted.insert(forward);
                drop(state);
                let (items, bytes) = self.budget.usage();
                let _previous = self.high_water_items.fetch_max(items, Ordering::Relaxed);
                let _previous = self.high_water_bytes.fetch_max(bytes, Ordering::Relaxed);
                let _counted = self.deferred_total.fetch_add(1, Ordering::Relaxed);
                Ok(permit)
            }
            Err(reason) => {
                let _counted = self.backlog_capacity_drops.fetch_add(1, Ordering::Relaxed);
                Err(reason)
            }
        }
    }

    fn enqueue(self: &Arc<Self>, job: DeferredAnalysisJob) {
        let forward = job.forward;
        let start_worker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _previous = state.jobs.insert(forward, job);
            if state.worker_running {
                false
            } else {
                state.worker_running = true;
                true
            }
        };
        self.notify.notify_waiters();
        if !start_worker {
            return;
        }
        let queue = Arc::clone(self);
        let task_guard = self.background.guard();
        drop(tokio::spawn(async move {
            let _task_guard = task_guard;
            queue.run().await;
        }));
    }

    fn next_ready(&self) -> DeferredAnalysisNext {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(forward) = state.admitted.first().copied() else {
            state.worker_running = false;
            drop(state);
            return DeferredAnalysisNext::Empty;
        };
        let Some(job) = state.jobs.remove(&forward) else {
            drop(state);
            return DeferredAnalysisNext::Waiting;
        };
        let _removed = state.admitted.remove(&forward);
        drop(state);
        DeferredAnalysisNext::Ready(Box::new(job))
    }

    fn cancel_admission(&self, forward: ForwardId) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _removed = state.admitted.remove(&forward);
        drop(state);
        self.notify.notify_waiters();
    }

    async fn run(self: Arc<Self>) {
        loop {
            self.wait_for_forwarding_idle().await;
            let notified = self.notify.notified();
            let job = match self.next_ready() {
                DeferredAnalysisNext::Ready(job) => *job,
                DeferredAnalysisNext::Waiting => {
                    notified.await;
                    continue;
                }
                DeferredAnalysisNext::Empty => return,
            };
            let DeferredAnalysisJob {
                forward,
                wire_body,
                content_encoding,
                decode_limits,
                context_limits,
                wire_content_hash,
                sink,
                admission,
                enqueued_at,
            } = job;
            let wait_us = u64::try_from(enqueued_at.elapsed().as_micros()).unwrap_or(u64::MAX);
            let _waited = self.analysis_wait_us.fetch_add(wait_us, Ordering::Relaxed);
            let outcome = tokio::task::spawn_blocking(move || {
                let _admission = admission;
                let decoded =
                    BoundedAnalysisDecoder.decode(&wire_body, content_encoding, &decode_limits);
                decoded.body.map_or_else(
                    || ContextAnalysisOutcome::Dropped(decode_drop_reason(decoded.status)),
                    |body| {
                        let mut analysis = analyze_responses(body.as_ref(), context_limits);
                        analysis.request_content_hash = wire_content_hash;
                        ContextAnalysisOutcome::Analyzed(analysis)
                    },
                )
            })
            .await
            .unwrap_or(ContextAnalysisOutcome::Dropped(
                ContextAnalysisDropReason::Cancelled,
            ));
            let _accepted = tokio::task::spawn_blocking(move || {
                sink.try_record_context_analysis(ContextAnalysisObservation { forward, outcome })
            })
            .await;
            let _counted = self
                .processed_deferred_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn wait_for_forwarding_idle(&self) {
        loop {
            if self.active_forwards.load(Ordering::Acquire) == 0 {
                return;
            }
            let notified = self.active_notify.notified();
            if self.active_forwards.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }

    fn metrics(&self) -> DeferredAnalysisMetrics {
        let (items, bytes) = self.budget.usage();
        DeferredAnalysisMetrics {
            queue_items: u64::try_from(items).unwrap_or(u64::MAX),
            queue_bytes: bytes,
            high_water_items: u64::try_from(self.high_water_items.load(Ordering::Relaxed))
                .unwrap_or(u64::MAX),
            high_water_bytes: self.high_water_bytes.load(Ordering::Relaxed),
            deferred_total: self.deferred_total.load(Ordering::Relaxed),
            processed_deferred_total: self.processed_deferred_total.load(Ordering::Relaxed),
            backlog_capacity_drops: self.backlog_capacity_drops.load(Ordering::Relaxed),
            analysis_wait_us: self.analysis_wait_us.load(Ordering::Relaxed),
        }
    }
}

/// Cloneable Axum state for one transparent ingress.
#[derive(Clone)]
pub struct TransparentProxy {
    client: reqwest::Client,
    config: ProxyConfig,
    metadata: Arc<dyn MetadataSink>,
    observations: Arc<dyn ProviderObservationSink>,
    observations_enabled: bool,
    deferred_analysis: Arc<DeferredAnalysisQueue>,
    observation_decode_permits: Arc<Semaphore>,
    forwards: Arc<AtomicU64>,
    active_forwards: Arc<AtomicU64>,
    active_notify: Arc<Notify>,
    background: BackgroundTracker,
}

impl std::fmt::Debug for TransparentProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransparentProxy")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl TransparentProxy {
    /// Creates a proxy using an HTTP/1.1-only rustls client.
    ///
    /// # Errors
    /// Returns a typed transport error when the client cannot be constructed.
    pub fn new(config: ProxyConfig) -> Result<Self, ForwardError> {
        let client = reqwest::Client::builder()
            .http1_only()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(ForwardError::ClientBuild)?;
        let background = BackgroundTracker::new();
        let active_forwards = Arc::new(AtomicU64::new(0));
        let active_notify = Arc::new(Notify::new());
        let deferred_analysis = DeferredAnalysisQueue::new(
            background.clone(),
            Arc::clone(&active_forwards),
            Arc::clone(&active_notify),
        );
        Ok(Self {
            observation_decode_permits: Arc::new(Semaphore::new(
                OBSERVATION_DECODE_CONCURRENCY_HARD_CAP,
            )),
            client,
            config,
            metadata: Arc::new(NoopMetadataSink),
            observations: Arc::new(NoopProviderObservationSink),
            observations_enabled: false,
            forwards: Arc::new(AtomicU64::new(0)),
            active_forwards,
            active_notify,
            background,
            deferred_analysis,
        })
    }

    /// Allocates the correlation identity of the next accepted forward.
    fn next_forward(&self) -> ForwardId {
        ForwardId(self.forwards.fetch_add(1, Ordering::Relaxed))
    }
    /// Marks one request as active until its response body or failure path settles.
    fn begin_forward(&self) -> ActiveForwardGuard {
        ActiveForwardGuard::acquire(&self.active_forwards, &self.active_notify)
    }
    /// Waits for detached provider/context observers before recorder shutdown.
    pub async fn wait_for_background_tasks(&self) {
        self.background.wait().await;
    }

    /// Returns bounded deferred-analysis queue counters for diagnostics and benchmarks.
    #[must_use]
    pub fn deferred_analysis_metrics(&self) -> DeferredAnalysisMetrics {
        self.deferred_analysis.metrics()
    }

    /// Returns a handle for auxiliary sink work that must drain before shutdown.
    #[must_use]
    pub fn background_task_spawner(&self) -> BackgroundTaskSpawner {
        BackgroundTaskSpawner {
            tracker: self.background.clone(),
        }
    }

    /// Installs an allowlisted metadata sink. Sink behavior never gates forwarding.
    #[must_use]
    pub fn with_metadata_sink(mut self, metadata: Arc<dyn MetadataSink>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Installs a non-blocking Responses v1 semantic observation sink. Sink behavior never gates
    /// forwarding.
    #[must_use]
    pub fn with_observation_sink(mut self, sink: Arc<dyn ProviderObservationSink>) -> Self {
        self.observations = sink;
        self.observations_enabled = true;
        self
    }

    /// Builds the transparent HTTP routes.
    pub fn router(self) -> Router {
        Router::new()
            .route("/v1/chat/completions", post(forward_chat))
            .route("/v1/responses", post(forward_responses))
            .route("/v1/responses/compact", post(forward_responses_compact))
            .with_state(self)
    }
}

async fn forward_chat(State(proxy): State<TransparentProxy>, request: Request) -> Response {
    forward(&proxy, request, InboundRoute::ChatCompletions).await
}

async fn forward_responses(State(proxy): State<TransparentProxy>, request: Request) -> Response {
    forward(&proxy, request, InboundRoute::Responses).await
}

async fn forward_responses_compact(
    State(proxy): State<TransparentProxy>,
    request: Request,
) -> Response {
    forward(&proxy, request, InboundRoute::ResponsesCompact).await
}

async fn forward(proxy: &TransparentProxy, request: Request, route: InboundRoute) -> Response {
    match forward_inner(proxy, request, route).await {
        Ok(response) => response,
        Err(ForwardError::RequestTooLarge) => error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body exceeds configured limit",
        ),
        Err(error) => error_response(StatusCode::BAD_GATEWAY, error.public_message()),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the forwarding state machine keeps request, response, and fail-open boundaries together"
)]
async fn forward_inner(
    proxy: &TransparentProxy,
    request: Request,
    route: InboundRoute,
) -> Result<Response, ForwardError> {
    let forward = proxy.next_forward();
    let forward_background = proxy.background.guard();
    let active_forward = proxy.begin_forward();
    let observed = proxy.observations_enabled && matches!(route, InboundRoute::Responses);
    let compaction = proxy.observations_enabled && matches!(route, InboundRoute::ResponsesCompact);
    let context_enabled = observed
        && matches!(
            proxy.config.context_analysis_mode,
            ContextAnalysisMode::Shadow
        );
    let (parts, body) = request.into_parts();

    let maximum = usize::try_from(proxy.config.max_request_body_bytes.get())
        .map_err(|_error| ForwardError::RequestLimitUnrepresentable)?;
    let bytes = to_bytes(body, maximum)
        .await
        .map_err(|_error| ForwardError::RequestTooLarge)?;
    let request_bytes =
        u64::try_from(bytes.len()).map_err(|_error| ForwardError::RequestLimitUnrepresentable)?;
    let wire_body = WireBody::new(bytes);
    let content_encoding =
        parse_content_encoding_header(parts.headers.get(axum::http::header::CONTENT_ENCODING));
    let headers = forwarded_headers(&parts.headers);
    let target = match upstream_uri(&proxy.config.upstream, route) {
        Ok(target) => target,
        Err(error) => {
            if compaction {
                queue_compaction_failure(
                    proxy,
                    forward,
                    request_bytes,
                    &wire_body,
                    content_encoding,
                    TransportFailure::Endpoint,
                );
            } else if observed {
                queue_transport_failure(proxy, forward, TransportFailure::Endpoint);
            }
            return Err(error);
        }
    };
    // Phase 2 request parsing may begin as soon as the body is available. Admission only
    // reserves bounded storage for the raw wire body; execution is deferred until the response
    // settles and never competes with forwarding for this reservation.
    let context_admission =
        context_enabled.then(|| proxy.deferred_analysis.try_admit(forward, request_bytes));
    let observation = observed
        .then(|| {
            queue_request_observation(RequestObservationRequest {
                proxy,
                forward,
                wire_body: wire_body.clone(),
                content_encoding,
                analysis_admission: context_admission,
            })
        })
        .flatten();
    let upstream = match proxy
        .client
        .post(target)
        .headers(headers)
        .body(wire_body.clone_bytes())
        .send()
        .await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            if context_enabled {
                if let Some(observation) = observation {
                    if let Some(trigger) =
                        queue_context_analysis(proxy, forward, observation).trigger
                    {
                        trigger.start();
                    }
                }
            }
            if compaction {
                queue_compaction_failure(
                    proxy,
                    forward,
                    request_bytes,
                    &wire_body,
                    content_encoding,
                    TransportFailure::classify(&error),
                );
            } else if observed {
                queue_transport_failure(proxy, forward, TransportFailure::classify(&error));
            }
            return Err(ForwardError::Upstream(error));
        }
    };
    let (stream_hint, context_trigger) = if context_enabled {
        observation.map_or((None, None), |observation| {
            let queued = queue_context_analysis(proxy, forward, observation);
            (Some(queued.stream_hint), queued.trigger)
        })
    } else {
        (None, None)
    };
    let status = upstream.status();
    let upstream_length = upstream.content_length();
    let metadata = ForwardMetadata {
        forward,
        route,
        request_bytes,
        status_code: Some(status.as_u16()),
        response_bytes: None,
    };
    if let Err(_error) = proxy.metadata.try_record(metadata) {
        // Transport metadata is also fail-open.
    }
    let response_headers = forwarded_headers(upstream.headers());
    let maximum_response = proxy.config.max_response_body_bytes.get();
    let selection = if observed {
        observation_limits(maximum_response)
            .and_then(|_limits| response_mode(&response_headers, stream_hint))
    } else {
        None
    };
    let body = if compaction {
        Body::from_stream(tapped_compaction_response_stream(
            upstream.bytes_stream(),
            CompactionTapConfig {
                forward,
                request_bytes,
                wire_sha256: Sha256::digest(wire_body.as_ref()).into(),
                content_encoding,
                transport: proxy.config.upstream.transport(),
                endpoint_profile_version: proxy.config.upstream.profile_version(),
                status_code: status.as_u16(),
                upstream_length,
                maximum_response,
                started: Instant::now(),
                sink: Arc::clone(&proxy.observations),
                background: proxy.background.clone(),
                background_guard: forward_background,
                active_forward: Some(active_forward),
            },
        ))
    } else if let Some(mode) = selection {
        Body::from_stream(tapped_response_stream(
            upstream.bytes_stream(),
            ResponseTapConfig {
                forward,
                maximum_response,
                upstream_length,
                mode,
                sink: Arc::clone(&proxy.observations),
                context_trigger,
                background: proxy.background.clone(),
                background_guard: forward_background,
                active_forward: Some(active_forward),
            },
        ))
    } else {
        Body::from_stream(bounded_response_stream(
            upstream.bytes_stream(),
            maximum_response,
            context_trigger,
            Some(active_forward),
            forward_background,
        ))
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok(response)
}

/// Forwards response bytes verbatim while enforcing the configured response bound.
#[allow(
    clippy::too_many_arguments,
    reason = "the bounded response stream receives its detached observer state explicitly"
)]
fn bounded_response_stream<StreamType>(
    source: StreamType,
    maximum_response: u64,
    context_trigger: Option<ContextAnalysisTrigger>,
    active_forward: Option<ActiveForwardGuard>,
    background_guard: BackgroundGuard,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>>
where
    StreamType: futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin + Send + 'static,
{
    stream::unfold(
        BoundedResponseState {
            source,
            context_trigger,
            active_forward,
            background_guard: Some(background_guard),
            maximum_response,
            seen: 0,
            finished: false,
        },
        |mut state| async move {
            let item = state.source.next().await;
            let next = match item {
                Some(Ok(chunk)) if !state.finished => match u64::try_from(chunk.len())
                    .ok()
                    .and_then(|length| state.seen.checked_add(length))
                {
                    Some(total) if total <= state.maximum_response => {
                        state.seen = total;
                        Some(Ok(chunk))
                    }
                    _ => {
                        state.finished = true;
                        Some(Err(std::io::Error::other(
                            "response body exceeds configured limit",
                        )))
                    }
                },
                Some(Err(error)) if !state.finished => {
                    state.finished = true;
                    Some(Err(std::io::Error::other(error)))
                }
                Some(Ok(_) | Err(_)) => None,
                None => {
                    state.finished = true;
                    None
                }
            };
            next.map(|item| (item, state))
        },
    )
}

struct BoundedResponseState<StreamType> {
    source: StreamType,
    context_trigger: Option<ContextAnalysisTrigger>,
    active_forward: Option<ActiveForwardGuard>,
    background_guard: Option<BackgroundGuard>,
    maximum_response: u64,
    seen: u64,
    finished: bool,
}

impl<StreamType> Drop for BoundedResponseState<StreamType> {
    fn drop(&mut self) {
        // The downstream may stop polling immediately after a complete Content-Length body, or
        // before the upstream yields its first chunk. Both are settled forwarding outcomes for
        // Phase 3: trigger only after this stream is no longer on the forwarding path.
        drop(self.active_forward.take());
        if let Some(trigger) = self.context_trigger.take() {
            trigger.start();
        }
        drop(self.background_guard.take());
    }
}

#[derive(Clone, Copy, Debug)]
enum ResponseObservationMode {
    Streaming,
    Json,
}

/// Observer selection for one forward, possibly still awaiting the request's stream hint.
enum ResponseModeSelection {
    /// The upstream content type already named the observer.
    Resolved(ResponseObservationMode),
    /// The upstream declared no content type, so the parsed request breaks the tie.
    RequestHint(tokio::sync::oneshot::Receiver<Option<bool>>),
}

fn observation_limits(max_semantic_bytes: u64) -> Option<ObservationLimits> {
    let max_semantic_bytes = usize::try_from(max_semantic_bytes).ok()?;
    ObservationLimits::new(ObservationLimitValues {
        max_semantic_bytes,
        // The retained usage object is bounded by the durable `provider_usage.raw_usage_json`
        // column, which the provider crate exports as the single value both sides derive from.
        max_usage_bytes: max_semantic_bytes.min(MAX_RETAINED_USAGE_BYTES),
        max_sse_event_bytes: max_semantic_bytes.min(1024 * 1024),
        max_sse_events: 100_000,
        max_json_depth: 64,
        max_json_items: 100_000,
        max_string_bytes: max_semantic_bytes.min(16 * 1024),
    })
    .ok()
}

/// Selects the response observer from what the upstream declared it sent.
///
/// The content type decides: `text/event-stream` selects the stream framer and JSON media types
/// select the document parser, so a JSON error body answering a streaming request keeps its
/// `error_code` instead of being fed to the framer. The request's `stream` hint only breaks the tie
/// when the upstream declared no content type at all.
fn response_mode(
    headers: &HeaderMap,
    stream_hint: Option<tokio::sync::oneshot::Receiver<Option<bool>>>,
) -> Option<ResponseModeSelection> {
    let declared = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or_default().trim());
    match declared {
        Some(content_type) if content_type.eq_ignore_ascii_case("text/event-stream") => Some(
            ResponseModeSelection::Resolved(ResponseObservationMode::Streaming),
        ),
        Some(content_type)
            if content_type.eq_ignore_ascii_case("application/json")
                || content_type.ends_with("+json") =>
        {
            Some(ResponseModeSelection::Resolved(
                ResponseObservationMode::Json,
            ))
        }
        Some(_unsupported) => None,
        None => stream_hint.map(ResponseModeSelection::RequestHint),
    }
}
/// Phase 2 parsing is detached as soon as the request body is available. Phase 3 analysis is
/// started later by [`queue_context_analysis`], after the upstream dispatch boundary.
struct QueuedRequestObservation {
    stream_hint: tokio::sync::oneshot::Receiver<Option<bool>>,
    analysis_input: tokio::sync::oneshot::Receiver<(WireBody, ContentEncoding)>,
    /// Present only after deferred raw-body admission succeeded.
    admission: Option<DeferredAnalysisPermit>,
    decode_limits: DecodeLimits,
}

struct RequestObservationRequest<'a> {
    proxy: &'a TransparentProxy,
    forward: ForwardId,
    wire_body: WireBody,
    content_encoding: tracepress_provider::ContentEncoding,
    analysis_admission: Option<Result<DeferredAnalysisPermit, ContextAnalysisDropReason>>,
}

#[allow(
    clippy::significant_drop_tightening,
    reason = "the decoder permit must live inside the detached observation task"
)]
fn queue_request_observation(
    request: RequestObservationRequest<'_>,
) -> Option<QueuedRequestObservation> {
    let RequestObservationRequest {
        proxy,
        forward,
        wire_body,
        content_encoding,
        analysis_admission,
    } = request;
    let limits = observation_limits(proxy.config.max_request_body_bytes.get())?;
    let decode_limits = DecodeLimits {
        max_compressed_bytes: proxy.config.max_request_body_bytes.get(),
        max_decompressed_bytes: proxy.config.resource_limits.max_decompressed_bytes.get(),
        max_expansion_ratio: None,
        max_decode_time: Duration::from_millis(
            proxy.config.resource_limits.max_processing_time_ms.get(),
        ),
    };
    let transport = proxy.config.upstream.transport();
    let endpoint_profile_version = proxy.config.upstream.profile_version();
    let sink = Arc::clone(&proxy.observations);
    let (hint, stream_hint) = tokio::sync::oneshot::channel();
    let (analysis_sender, analysis_input) = tokio::sync::oneshot::channel();
    let decode_permit = match content_encoding {
        ContentEncoding::Zstd => Arc::clone(&proxy.observation_decode_permits)
            .try_acquire_owned()
            .ok(),
        _ => None,
    };
    let skip_decode = matches!(content_encoding, ContentEncoding::Zstd) && decode_permit.is_none();
    let (admission, context) = match analysis_admission {
        Some(Ok(admission)) => (Some(admission), None),
        Some(Err(reason)) => (None, Some(ContextAnalysisOutcome::Dropped(reason))),
        None => (None, None),
    };
    let deferred_wire_body = wire_body.clone();
    let observation_decode_limits = decode_limits.clone();
    let task_guard = proxy.background.guard();
    drop(tokio::task::spawn_blocking(move || {
        let _task_guard = task_guard;
        let _decode_permit = decode_permit;
        let decoded = if skip_decode {
            DecodeResult {
                status: AnalysisDecodeStatus::ResourceLimit,
                body: None,
                wire_bytes: u64::try_from(wire_body.len()).unwrap_or(u64::MAX),
                decoded_bytes: 0,
                decode_duration_us: 0,
                decoder_version: 1,
            }
        } else {
            BoundedAnalysisDecoder.decode(&wire_body, content_encoding, &observation_decode_limits)
        };
        let mut parsed = if let Some(body) = decoded.body.as_ref() {
            OpenAiResponsesV1Observer::new()
                .observe_request(ObservationInput::new(body.as_ref(), limits))
                .unwrap_or_else(|_error| {
                    tracepress_provider::parse_request(ObservationInput::new(body.as_ref(), limits))
                })
        } else {
            RequestObservation::unavailable(observation_status_for_decode(decoded.status))
        };
        parsed.transport = transport;
        parsed.endpoint_profile_version = Some(endpoint_profile_version);
        parsed.request_bytes = decoded.body.as_ref().map(|_body| decoded.decoded_bytes);
        parsed.wire_bytes = Some(decoded.wire_bytes);
        parsed.wire_sha256 = Some(
            Sha256::digest(wire_body.as_ref())
                .to_vec()
                .into_boxed_slice(),
        );
        parsed.decoded_bytes = Some(decoded.decoded_bytes);
        parsed.content_encoding = content_encoding;
        parsed.analysis_decode_status = decoded.status;
        parsed.decode_duration_us = Some(decoded.decode_duration_us);
        parsed.decoder_version = Some(decoded.decoder_version);
        let _accepted = sink.try_record_request_context(RequestContextObservation {
            forward,
            observation: parsed.clone(),
            context,
        });
        let _delivered = hint.send(parsed.stream);
        let _sent = analysis_sender.send((deferred_wire_body, content_encoding));
    }));
    Some(QueuedRequestObservation {
        stream_hint,
        analysis_input,
        admission,
        decode_limits,
    })
}

/// Defers Phase 3 analysis until the response body has settled.
struct QueuedContextAnalysis {
    stream_hint: tokio::sync::oneshot::Receiver<Option<bool>>,
    trigger: Option<ContextAnalysisTrigger>,
}

struct ContextAnalysisTrigger {
    forward: ForwardId,
    analysis_input: tokio::sync::oneshot::Receiver<(WireBody, ContentEncoding)>,
    context_limits: ContextAnalysisLimits,
    decode_limits: DecodeLimits,
    admission: DeferredAnalysisPermit,
    sink: Arc<dyn ProviderObservationSink>,
    deferred_analysis: Arc<DeferredAnalysisQueue>,
    background: BackgroundTracker,
}

impl ContextAnalysisTrigger {
    fn start(self) {
        let Self {
            forward,
            analysis_input,
            context_limits,
            decode_limits,
            admission,
            sink,
            deferred_analysis,
            background,
        } = self;
        let task_guard = background.guard();
        drop(tokio::spawn(async move {
            let _task_guard = task_guard;
            let Ok((wire_body, content_encoding)) = analysis_input.await else {
                let _accepted = tokio::task::spawn_blocking(move || {
                    sink.try_record_context_analysis(ContextAnalysisObservation {
                        forward,
                        outcome: ContextAnalysisOutcome::Dropped(
                            ContextAnalysisDropReason::Cancelled,
                        ),
                    })
                })
                .await;
                drop(admission);
                return;
            };
            tokio::time::sleep(CONTEXT_ANALYSIS_QUIESCENCE_WINDOW).await;
            let wire_content_hash = ContextDigest::from_bytes(wire_body.as_ref());
            deferred_analysis.enqueue(DeferredAnalysisJob {
                forward,
                wire_body,
                content_encoding,
                decode_limits,
                context_limits,
                wire_content_hash,
                sink,
                admission,
                enqueued_at: Instant::now(),
            });
        }));
    }
}

const fn observation_status_for_decode(status: AnalysisDecodeStatus) -> ObservationStatus {
    match status {
        AnalysisDecodeStatus::Identity | AnalysisDecodeStatus::Decoded => {
            ObservationStatus::Complete
        }
        AnalysisDecodeStatus::CorruptPayload => ObservationStatus::Malformed,
        AnalysisDecodeStatus::ResourceLimit | AnalysisDecodeStatus::Timeout => {
            ObservationStatus::ResourceLimit
        }
        _ => ObservationStatus::Unsupported,
    }
}

const fn decode_drop_reason(status: AnalysisDecodeStatus) -> ContextAnalysisDropReason {
    match status {
        AnalysisDecodeStatus::Identity | AnalysisDecodeStatus::Decoded => {
            ContextAnalysisDropReason::Malformed
        }
        AnalysisDecodeStatus::CorruptPayload => ContextAnalysisDropReason::Malformed,
        AnalysisDecodeStatus::ResourceLimit | AnalysisDecodeStatus::Timeout => {
            ContextAnalysisDropReason::ResourceLimit
        }
        _ => ContextAnalysisDropReason::Unsupported,
    }
}

/// Prepares Phase 3 analysis without starting it. The trigger is started by the response body
/// after forwarding settles, including an empty body, error, or downstream cancellation.
fn queue_context_analysis(
    proxy: &TransparentProxy,
    forward: ForwardId,
    queued: QueuedRequestObservation,
) -> QueuedContextAnalysis {
    let QueuedRequestObservation {
        stream_hint,
        analysis_input,
        admission,
        decode_limits,
    } = queued;
    let trigger = admission.map(|admission| ContextAnalysisTrigger {
        forward,
        analysis_input,
        context_limits: proxy.config.context_analysis_limits,
        decode_limits,
        admission,
        sink: Arc::clone(&proxy.observations),
        deferred_analysis: Arc::clone(&proxy.deferred_analysis),
        background: proxy.background.clone(),
    });
    QueuedContextAnalysis {
        stream_hint,
        trigger,
    }
}

/// Reports a forward that obtained no upstream response, without blocking the failing task.
fn queue_transport_failure(
    proxy: &TransparentProxy,
    forward: ForwardId,
    failure: TransportFailure,
) {
    let sink = Arc::clone(&proxy.observations);
    let background = proxy.background.clone();
    let task_guard = background.guard();
    drop(tokio::task::spawn_blocking(move || {
        let _task_guard = task_guard;
        sink.try_record_transport_failure(forward, failure)
    }));
}

/// Records a compact transport failure without retaining any request or response bytes.
#[allow(
    clippy::too_many_arguments,
    reason = "the failure record carries the bounded transport facts without retaining a payload"
)]
fn queue_compaction_failure(
    proxy: &TransparentProxy,
    forward: ForwardId,
    request_bytes: u64,
    wire_body: &WireBody,
    content_encoding: ContentEncoding,
    failure: TransportFailure,
) {
    let observation = CompactionObservation {
        forward,
        request_bytes,
        wire_sha256: Sha256::digest(wire_body.as_ref()).into(),
        content_encoding,
        transport: proxy.config.upstream.transport(),
        endpoint_profile_version: proxy.config.upstream.profile_version(),
        status_code: None,
        response_bytes: 0,
        duration_us: None,
        outcome: CompactionOutcome::Failed,
        transport_failure: Some(failure),
    };
    queue_compaction_observation(
        Arc::clone(&proxy.observations),
        &proxy.background,
        observation,
    );
}

/// Hands compact transport evidence to the sink on a detached blocking auxiliary task.
///
/// The CLI sink uses a bounded durable send here. Keeping the wait in the auxiliary task preserves
/// forwarding latency while ensuring a full recorder queue is backpressured and counted at the
/// shutdown boundary instead of silently dropping compact evidence.
fn queue_compaction_observation(
    sink: Arc<dyn ProviderObservationSink>,
    background: &BackgroundTracker,
    observation: CompactionObservation,
) {
    let task_guard = background.guard();
    drop(tokio::task::spawn_blocking(move || {
        let _task_guard = task_guard;
        let _accepted = sink.try_record_compaction(observation);
    }));
}

struct CompactionTapConfig {
    forward: ForwardId,
    request_bytes: u64,
    wire_sha256: [u8; 32],
    content_encoding: ContentEncoding,
    transport: ProviderTransport,
    endpoint_profile_version: u32,
    status_code: u16,
    upstream_length: Option<u64>,
    maximum_response: u64,
    started: Instant,
    sink: Arc<dyn ProviderObservationSink>,
    background: BackgroundTracker,
    background_guard: BackgroundGuard,
    active_forward: Option<ActiveForwardGuard>,
}

/// Forwards a compaction response byte-for-byte while measuring only bounded transport facts.
fn tapped_compaction_response_stream<S>(
    source: S,
    config: CompactionTapConfig,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>>
where
    S: futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin + Send + 'static,
{
    stream::unfold(
        CompactionTapState {
            source,
            forward: config.forward,
            request_bytes: config.request_bytes,
            wire_sha256: config.wire_sha256,
            content_encoding: config.content_encoding,
            transport: config.transport,
            endpoint_profile_version: config.endpoint_profile_version,
            status_code: config.status_code,
            upstream_length: config.upstream_length,
            maximum_response: config.maximum_response,
            started: config.started,
            sink: config.sink,
            background: config.background,
            background_guard: Some(config.background_guard),
            active_forward: config.active_forward,
            seen: 0,
            settled: false,
        },
        |mut state| async move {
            match state.source.next().await {
                Some(Ok(chunk)) if !state.settled => {
                    let length = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
                    let Some(total) = state.seen.checked_add(length) else {
                        state.settle(CompactionOutcome::Incomplete, None);
                        return Some((
                            Err(std::io::Error::other(
                                "response body exceeds configured limit",
                            )),
                            state,
                        ));
                    };
                    if total > state.maximum_response {
                        state.settle(CompactionOutcome::Incomplete, None);
                        return Some((
                            Err(std::io::Error::other(
                                "response body exceeds configured limit",
                            )),
                            state,
                        ));
                    }
                    state.seen = total;
                    Some((Ok(chunk), state))
                }
                Some(Ok(_chunk)) => None,
                Some(Err(error)) if !state.settled => {
                    state.settle(CompactionOutcome::Disconnected, None);
                    Some((Err(std::io::Error::other(error)), state))
                }
                Some(Err(_error)) => None,
                None if !state.settled => {
                    let outcome = if (200..300).contains(&state.status_code) {
                        CompactionOutcome::Completed
                    } else {
                        CompactionOutcome::Failed
                    };
                    state.settle(outcome, None);
                    None
                }
                None => None,
            }
        },
    )
}

struct CompactionTapState<StreamType> {
    source: StreamType,
    forward: ForwardId,
    request_bytes: u64,
    wire_sha256: [u8; 32],
    content_encoding: ContentEncoding,
    transport: ProviderTransport,
    endpoint_profile_version: u32,
    status_code: u16,
    upstream_length: Option<u64>,
    maximum_response: u64,
    started: Instant,
    sink: Arc<dyn ProviderObservationSink>,
    background: BackgroundTracker,
    background_guard: Option<BackgroundGuard>,
    active_forward: Option<ActiveForwardGuard>,
    seen: u64,
    settled: bool,
}

impl<StreamType> CompactionTapState<StreamType> {
    fn settle(&mut self, outcome: CompactionOutcome, transport_failure: Option<TransportFailure>) {
        if self.settled {
            return;
        }
        self.settled = true;
        let observation = CompactionObservation {
            forward: self.forward,
            request_bytes: self.request_bytes,
            wire_sha256: self.wire_sha256,
            content_encoding: self.content_encoding,
            transport: self.transport,
            endpoint_profile_version: self.endpoint_profile_version,
            status_code: Some(self.status_code),
            response_bytes: self.seen,
            duration_us: elapsed_us(self.started),
            outcome,
            transport_failure,
        };
        queue_compaction_observation(Arc::clone(&self.sink), &self.background, observation);
        drop(self.active_forward.take());
        drop(self.background_guard.take());
    }
}

impl<StreamType> Drop for CompactionTapState<StreamType> {
    fn drop(&mut self) {
        if !self.settled {
            let outcome = if self.upstream_length == Some(self.seen) {
                if (200..300).contains(&self.status_code) {
                    CompactionOutcome::Completed
                } else {
                    CompactionOutcome::Failed
                }
            } else {
                CompactionOutcome::Cancelled
            };
            self.settle(outcome, None);
        }
    }
}

enum ObservationMessage {
    Chunk(Bytes),
    Finished,
    Disconnected,
}

fn queue_observation(
    sender: &mut Option<tokio::sync::mpsc::Sender<ObservationMessage>>,
    dropped: &Arc<AtomicBool>,
    message: ObservationMessage,
) {
    if let Some(sender) = sender {
        if sender.try_send(message).is_err() {
            dropped.store(true, Ordering::Relaxed);
        }
    }
}

struct ResponseTapConfig {
    forward: ForwardId,
    maximum_response: u64,
    /// Exact upstream body length, when the upstream advertised one.
    upstream_length: Option<u64>,
    mode: ResponseModeSelection,
    sink: Arc<dyn ProviderObservationSink>,
    context_trigger: Option<ContextAnalysisTrigger>,
    active_forward: Option<ActiveForwardGuard>,
    background_guard: BackgroundGuard,
    background: BackgroundTracker,
}
#[allow(
    clippy::too_many_lines,
    reason = "the stream state machine keeps byte forwarding and terminal observer signaling together"
)]
fn tapped_response_stream<S>(
    source: S,
    config: ResponseTapConfig,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>>
where
    S: futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin + Send + 'static,
{
    let limits = observation_limits(config.maximum_response).unwrap_or_else(|| {
        // The caller only enables this path when the configured bound is representable.
        ObservationLimits::default()
    });
    let (sender, receiver) = tokio::sync::mpsc::channel(OBSERVATION_QUEUE_CAPACITY);
    let dropped = Arc::new(AtomicBool::new(false));
    let worker_dropped = Arc::clone(&dropped);
    let background = config.background.clone();
    let task_guard = background.guard();
    let _response_observer_task = tokio::spawn(async move {
        let _task_guard = task_guard;
        observe_response_task(ResponseObservationTask {
            receiver,
            forward: config.forward,
            sink: config.sink,
            selection: config.mode,
            limits,
            dropped: worker_dropped,
        })
        .await;
    });
    let state = TapState {
        sender: Some(sender),
        dropped,
        maximum_response: config.maximum_response,
        upstream_length: config.upstream_length,
        context_trigger: config.context_trigger,
        active_forward: config.active_forward,
        background_guard: Some(config.background_guard),
        seen: 0,
        settled: false,
    };
    stream::unfold((source, state), |(mut source, mut state)| async move {
        match source.next().await {
            Some(Ok(chunk)) if !state.settled => {
                let length = u64::try_from(chunk.len()).unwrap_or_default();
                let Some(total) = state.seen.checked_add(length) else {
                    state.settled = true;
                    queue_observation(
                        &mut state.sender,
                        &state.dropped,
                        ObservationMessage::Disconnected,
                    );
                    drop(state.sender.take());
                    return Some((
                        Err(std::io::Error::other(
                            "response body exceeds configured limit",
                        )),
                        (source, state),
                    ));
                };
                if total > state.maximum_response {
                    state.settled = true;
                    queue_observation(
                        &mut state.sender,
                        &state.dropped,
                        ObservationMessage::Disconnected,
                    );
                    drop(state.sender.take());
                    return Some((
                        Err(std::io::Error::other(
                            "response body exceeds configured limit",
                        )),
                        (source, state),
                    ));
                }
                state.seen = total;
                queue_observation(
                    &mut state.sender,
                    &state.dropped,
                    ObservationMessage::Chunk(chunk.clone()),
                );
                Some((Ok(chunk), (source, state)))
            }
            Some(Ok(_chunk)) => None,
            Some(Err(error)) if !state.settled => {
                state.settled = true;
                queue_observation(
                    &mut state.sender,
                    &state.dropped,
                    ObservationMessage::Disconnected,
                );
                drop(state.sender.take());
                Some((Err(std::io::Error::other(error)), (source, state)))
            }
            Some(Err(_error)) => None,
            None => {
                state.settled = true;
                queue_observation(
                    &mut state.sender,
                    &state.dropped,
                    ObservationMessage::Finished,
                );
                drop(state.sender.take());
                None
            }
        }
    })
}

struct TapState {
    sender: Option<tokio::sync::mpsc::Sender<ObservationMessage>>,
    dropped: Arc<AtomicBool>,
    maximum_response: u64,
    upstream_length: Option<u64>,
    context_trigger: Option<ContextAnalysisTrigger>,
    active_forward: Option<ActiveForwardGuard>,
    background_guard: Option<BackgroundGuard>,
    seen: u64,
    settled: bool,
}

impl Drop for TapState {
    fn drop(&mut self) {
        if !self.settled {
            // A consumer stops polling a `Content-Length` body as soon as every advertised byte
            // was forwarded, so exhausting that length is a completed forward, not an abort.
            let message = if self.upstream_length == Some(self.seen) {
                ObservationMessage::Finished
            } else {
                ObservationMessage::Disconnected
            };
            queue_observation(&mut self.sender, &self.dropped, message);
            drop(self.sender.take());
        }
        // The body is no longer on the forwarding path. This also covers an empty body and a
        // downstream cancellation before the first chunk. Keeping this in Drop means an error
        // item is delivered before analysis can start.
        drop(self.active_forward.take());
        if let Some(trigger) = self.context_trigger.take() {
            trigger.start();
        }
        drop(self.background_guard.take());
    }
}

struct ResponseObservationTask {
    receiver: tokio::sync::mpsc::Receiver<ObservationMessage>,
    forward: ForwardId,
    sink: Arc<dyn ProviderObservationSink>,
    selection: ResponseModeSelection,
    limits: ObservationLimits,
    dropped: Arc<AtomicBool>,
}

/// Observes one response body off the forwarding task, measuring its own elapsed time.
///
/// The clock starts when this detached task starts, so every timing is a real local
/// measurement of the upstream exchange taken without any clock read on the forwarding path.
/// The streaming observer measures itself; the JSON document path is measured here, because it
/// only learns of the first upstream byte and of the terminal decision through this channel.
async fn observe_response_task(task: ResponseObservationTask) {
    let clock = Instant::now();
    let ResponseObservationTask {
        mut receiver,
        forward,
        sink,
        selection,
        limits,
        dropped,
    } = task;
    let Some(mode) = resolve_mode(selection).await else {
        // Neither the upstream nor the request said which observer applies to these bytes.
        return;
    };
    let mut streaming = (matches!(mode, ResponseObservationMode::Streaming))
        .then(|| StreamingObserver::new(limits));
    let mut body = Vec::new();
    let mut terminal = None;
    let mut first_byte_us = None;
    while let Some(message) = receiver.recv().await {
        match message {
            ObservationMessage::Chunk(chunk) => {
                if let Some(observer) = streaming.as_mut() {
                    let _result = observer.observe_chunk(&chunk);
                } else if body
                    .len()
                    .checked_add(chunk.len())
                    .is_some_and(|length| length <= limits.max_semantic_bytes)
                {
                    if first_byte_us.is_none() && !chunk.is_empty() {
                        first_byte_us = elapsed_us(clock);
                    }
                    body.extend_from_slice(&chunk);
                } else {
                    dropped.store(true, Ordering::Relaxed);
                }
            }
            ObservationMessage::Finished => {
                terminal = Some(ResponseTerminal::Finished);
                break;
            }
            ObservationMessage::Disconnected => {
                terminal = Some(ResponseTerminal::Disconnected);
                break;
            }
        }
    }
    // The terminal decision is timed before any parsing, so the duration reports the upstream
    // exchange rather than the observer's own work.
    let duration_us = elapsed_us(clock);
    let mut response = if let Some(mut observer) = streaming {
        match terminal {
            Some(ResponseTerminal::Finished) => observer.finish(),
            Some(ResponseTerminal::Disconnected) => observer.disconnect(),
            None => observer.cancel(),
        }
    } else {
        // A buffered document is parsed under `max_semantic_bytes`, which `tracepress run`
        // configures at 32 MiB: that is the same CPU-bound work the request half was detached
        // for, so it runs on a blocking task instead of occupying a runtime worker the
        // forwarding tasks need. Every timing was measured above, before this parse.
        let parsed = tokio::task::spawn_blocking(move || {
            observe_document(DocumentObservation {
                body,
                limits,
                terminal,
                first_byte_us,
                duration_us,
            })
        });
        let Ok(response) = parsed.await else {
            // A blocking task only fails while the runtime is shutting down. Observation is
            // auxiliary, so the forward keeps the evidence its other halves already left.
            return;
        };
        response
    };
    if dropped.load(Ordering::Relaxed) {
        response.status = ObservationStatus::ObserverBackpressure;
    }
    let _ = tokio::task::spawn_blocking(move || sink.try_record_response(forward, response)).await;
}

/// One buffered document response, with the timings measured around its bounded parse.
struct DocumentObservation {
    body: Vec<u8>,
    limits: ObservationLimits,
    terminal: Option<ResponseTerminal>,
    first_byte_us: Option<u64>,
    duration_us: Option<u64>,
}

/// Parses one buffered document response, then applies the already measured timings.
fn observe_document(observation: DocumentObservation) -> ResponseObservation {
    let DocumentObservation {
        body,
        limits,
        terminal,
        first_byte_us,
        duration_us,
    } = observation;
    let mut response = OpenAiResponsesV1Observer::new()
        .observe_response(ObservationInput::new(&body, limits))
        .unwrap_or_else(|_error| {
            tracepress_provider::parse_response(ObservationInput::new(&body, limits))
        });
    if matches!(terminal, Some(ResponseTerminal::Disconnected) | None) {
        response.status = if terminal.is_none() {
            ObservationStatus::Cancelled
        } else {
            ObservationStatus::Partial
        };
    }
    // A document response has no semantic output events, so TTFT stays unknown.
    response.ttfb_us = first_byte_us;
    response.duration_us = duration_us;
    response
}

/// Elapsed local microseconds since observation started, when representable.
///
/// A measurement the local clock cannot represent stays absent rather than becoming a
/// placeholder value.
fn elapsed_us(clock: Instant) -> Option<u64> {
    u64::try_from(clock.elapsed().as_micros()).ok()
}

/// Resolves the selected observer, awaiting the request's stream hint only when it decides.
async fn resolve_mode(selection: ResponseModeSelection) -> Option<ResponseObservationMode> {
    match selection {
        ResponseModeSelection::Resolved(mode) => Some(mode),
        ResponseModeSelection::RequestHint(receiver) => match receiver.await {
            Ok(Some(true)) => Some(ResponseObservationMode::Streaming),
            Ok(Some(false)) => Some(ResponseObservationMode::Json),
            Ok(None) | Err(_) => None,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResponseTerminal {
    Finished,
    Disconnected,
}

fn upstream_uri(
    endpoint: &ProviderEndpoint,
    route: InboundRoute,
) -> Result<reqwest::Url, ForwardError> {
    let validated_uri = endpoint
        .as_str()
        .parse::<Uri>()
        .map_err(|_error| ForwardError::EndpointUri)?;
    if validated_uri.scheme_str().is_none() || validated_uri.authority().is_none() {
        return Err(ForwardError::EndpointUri);
    }
    let mut url =
        reqwest::Url::parse(endpoint.as_str()).map_err(|_error| ForwardError::EndpointUri)?;
    let path = endpoint
        .upstream_path(route.path())
        .map_err(|_error| ForwardError::EndpointUri)?;
    url.set_path(path);
    Ok(url)
}

fn forwarded_headers(source: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::with_capacity(source.len());
    for (name, value) in source {
        if !is_hop_by_hop(name) && *name != axum::http::header::HOST {
            let _replaced = headers.append(name, value.clone());
        }
    }
    headers
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn error_response(status: StatusCode, message: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )
        .body(Body::from(message))
        .unwrap_or_else(|_error| Response::new(Body::empty()))
}

/// Transparent forwarding failure that contains no request headers or body bytes.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ForwardError {
    /// HTTP client construction failed.
    #[error("unable to construct upstream client")]
    ClientBuild(#[source] reqwest::Error),
    /// Request body exceeded its configured bound.
    #[error("request body exceeds configured limit")]
    RequestTooLarge,
    /// Configured request limit cannot be represented by this platform.
    #[error("request body limit is not representable")]
    RequestLimitUnrepresentable,
    /// The validated endpoint could not be rewritten for the inbound route.
    #[error("provider endpoint URI could not be rewritten")]
    EndpointUri,
    /// Upstream transport failed before a response was available.
    #[error("upstream transport failed")]
    Upstream(#[source] reqwest::Error),
}

impl ForwardError {
    const fn public_message(&self) -> &'static str {
        match self {
            Self::RequestTooLarge => "request body exceeds configured limit",
            Self::ClientBuild(_)
            | Self::RequestLimitUnrepresentable
            | Self::EndpointUri
            | Self::Upstream(_) => "upstream transport failed",
        }
    }
}

#[allow(
    dead_code,
    clippy::expect_used,
    clippy::panic,
    clippy::significant_drop_in_scrutinee,
    clippy::wildcard_imports,
    reason = "unit-test fixtures use compact assertions and shared module imports"
)]
mod tests {
    use super::*;

    #[test]
    fn saturated_observation_queue_is_nonblocking_and_fail_open() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        assert!(
            sender.try_send(ObservationMessage::Finished).is_ok(),
            "queue setup should fit"
        );
        let dropped = Arc::new(AtomicBool::new(false));
        let mut sender = Some(sender);

        queue_observation(
            &mut sender,
            &dropped,
            ObservationMessage::Chunk(Bytes::from_static(b"dropped")),
        );

        assert!(dropped.load(Ordering::Relaxed));
        assert!(matches!(
            receiver.try_recv(),
            Ok(ObservationMessage::Finished)
        ));
    }

    #[test]
    fn retained_usage_never_exceeds_the_durable_usage_column_bound() {
        // `provider_usage.raw_usage_json` is declared CHECK(length(raw_usage_json) <= 65536): a
        // usage object the parser accepts above that bound would abort the whole durable batch.
        const COLUMN_BOUND: usize = 65_536;

        for configured in [4_096_u64, 64 * 1024, 8 * 1024 * 1024, 32 * 1024 * 1024] {
            assert!(
                observation_limits(configured).is_some_and(|limits| {
                    limits.max_usage_bytes <= COLUMN_BOUND
                        && limits.max_usage_bytes
                            == usize::try_from(configured)
                                .unwrap_or(COLUMN_BOUND)
                                .min(COLUMN_BOUND)
                }),
                "usage bound for a {configured} byte body must stay inside the column bound"
            );
        }
    }

    #[test]
    fn subscription_upstream_uri_is_allowlisted_and_path_rewritten() {
        let endpoint = ProviderEndpoint::chatgpt_codex_subscription();
        let uri = upstream_uri(&endpoint, InboundRoute::Responses).expect("fixed endpoint");
        assert_eq!(
            uri.as_str(),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert!(upstream_uri(&endpoint, InboundRoute::ChatCompletions).is_err());
    }
    fn resource_limits_with_cpu_work_units(
        cpu_work_units: i128,
    ) -> Result<ResourceLimits, tracepress_core::ResourceLimitsError> {
        ResourceLimits::try_from(tracepress_core::ResourceLimitsConfig {
            max_raw_bytes: Some(8 * 1024 * 1024),
            max_request_body_bytes: Some(8 * 1024 * 1024),
            max_response_body_bytes: Some(32 * 1024 * 1024),
            max_decompressed_bytes: Some(32 * 1024 * 1024),
            max_ipc_frame_bytes: Some(65_536),
            max_ipc_queue_items: Some(128),
            max_json_nesting: Some(64),
            max_json_items: Some(100_000),
            max_line_bytes: Some(8 * 1024 * 1024),
            max_processing_time_ms: Some(250),
            max_cpu_work_units: Some(cpu_work_units),
        })
    }

    #[test]
    fn context_analysis_concurrency_has_a_deterministic_hard_cap()
    -> Result<(), Box<dyn std::error::Error>> {
        let limits = resource_limits_with_cpu_work_units(1_000_000_000)?;
        let analysis_limits =
            ContextAnalysisLimits::new(tracepress_context::ContextAnalysisLimitValues {
                max_analyzed_bytes: 1,
                max_blocks: 1,
                max_json_depth: 1,
                max_string_bytes_inspected: 1,
                max_analysis_work_units: 1,
                max_analysis_wall_time_ms: 1,
                max_batches: 1,
            })?;
        assert_eq!(
            derive_context_analysis_concurrency_with_parallelism(&limits, analysis_limits, 128)
                .get(),
            CONTEXT_ANALYSIS_CONCURRENCY_HARD_CAP
        );
        assert_eq!(
            derive_context_analysis_concurrency_with_parallelism(&limits, analysis_limits, 0).get(),
            1
        );
        Ok(())
    }

    #[test]
    fn deferred_analysis_budget_is_bounded_by_items_and_wire_bytes() {
        let budget = Arc::new(DeferredAnalysisBudget::new(2, 10));
        let first = budget.try_reserve(6).expect("first body should fit");
        assert!(matches!(
            budget.try_reserve(5),
            Err(ContextAnalysisDropReason::DeferredBacklogCapacity)
        ));
        let second = budget.try_reserve(4).expect("second body should fit");
        assert_eq!(budget.usage(), (2, 10));
        assert!(matches!(
            budget.try_reserve(0),
            Err(ContextAnalysisDropReason::DeferredBacklogCapacity)
        ));
        drop(first);
        assert_eq!(budget.usage(), (1, 4));
        drop(second);
        assert_eq!(budget.usage(), (0, 0));
    }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TriggerResult {
        Analyzed,
        Dropped(ContextAnalysisDropReason),
    }

    #[derive(Default)]
    struct TriggerSink {
        results: std::sync::Mutex<Vec<TriggerResult>>,
    }

    impl ProviderObservationSink for TriggerSink {
        fn try_record_request_context(
            &self,
            _observation: RequestContextObservation,
        ) -> Result<(), ObservationSinkError> {
            Ok(())
        }

        fn try_record_context_analysis(
            &self,
            observation: ContextAnalysisObservation,
        ) -> Result<(), ObservationSinkError> {
            let result = match observation.outcome {
                ContextAnalysisOutcome::Analyzed(_) => TriggerResult::Analyzed,
                ContextAnalysisOutcome::Dropped(reason) => TriggerResult::Dropped(reason),
            };
            self.results.lock().expect("test sink lock").push(result);
            Ok(())
        }

        fn try_record_response(
            &self,
            _forward: ForwardId,
            _observation: ResponseObservation,
        ) -> Result<(), ObservationSinkError> {
            Ok(())
        }

        fn try_record_transport_failure(
            &self,
            _forward: ForwardId,
            _failure: TransportFailure,
        ) -> Result<(), ObservationSinkError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct OrderingSink {
        forwards: std::sync::Mutex<Vec<ForwardId>>,
    }

    impl ProviderObservationSink for OrderingSink {
        fn try_record_request_context(
            &self,
            _observation: RequestContextObservation,
        ) -> Result<(), ObservationSinkError> {
            Ok(())
        }

        fn try_record_context_analysis(
            &self,
            observation: ContextAnalysisObservation,
        ) -> Result<(), ObservationSinkError> {
            self.forwards
                .lock()
                .expect("ordering sink lock")
                .push(observation.forward);
            Ok(())
        }

        fn try_record_response(
            &self,
            _forward: ForwardId,
            _observation: ResponseObservation,
        ) -> Result<(), ObservationSinkError> {
            Ok(())
        }

        fn try_record_transport_failure(
            &self,
            _forward: ForwardId,
            _failure: TransportFailure,
        ) -> Result<(), ObservationSinkError> {
            Ok(())
        }
    }

    fn test_context_trigger(
        active_forwards: &Arc<AtomicU64>,
        active_notify: &Arc<Notify>,
        sink: Arc<TriggerSink>,
    ) -> ContextAnalysisTrigger {
        let bytes = Bytes::from_static(br#"{"model":"test","input":"value"}"#);
        let queue = DeferredAnalysisQueue::new(
            BackgroundTracker::new(),
            Arc::clone(active_forwards),
            Arc::clone(active_notify),
        );
        let admission = queue
            .try_admit(
                ForwardId(0),
                u64::try_from(bytes.len()).expect("test body length"),
            )
            .expect("test analysis admission");
        let (sender, analysis_input) = tokio::sync::oneshot::channel();
        sender
            .send((WireBody::new(bytes), ContentEncoding::Identity))
            .expect("observation receiver is live");
        let context_limits =
            ContextAnalysisLimits::new(tracepress_context::ContextAnalysisLimitValues {
                max_analyzed_bytes: 4_096,
                max_blocks: 64,
                max_json_depth: 64,
                max_string_bytes_inspected: 4_096,
                max_analysis_work_units: 100_000,
                max_analysis_wall_time_ms: 250,
                max_batches: 1,
            })
            .expect("context limits");
        ContextAnalysisTrigger {
            forward: ForwardId(0),
            analysis_input,
            context_limits,
            decode_limits: DecodeLimits {
                max_compressed_bytes: 4096,
                max_decompressed_bytes: 4096,
                max_expansion_ratio: None,
                max_decode_time: Duration::from_secs(1),
            },
            admission,
            sink,
            deferred_analysis: queue,
            background: BackgroundTracker::new(),
        }
    }

    async fn wait_for_trigger_result(sink: &TriggerSink) -> TriggerResult {
        for _ in 0..1_000 {
            if let Some(result) = sink
                .results
                .lock()
                .expect("test sink lock")
                .first()
                .copied()
            {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("context trigger did not report a result");
    }

    fn deferred_job(
        queue: &Arc<DeferredAnalysisQueue>,
        forward: ForwardId,
        sink: Arc<OrderingSink>,
    ) -> DeferredAnalysisJob {
        let bytes = Bytes::from_static(br#"{"model":"test","input":"value"}"#);
        let wire_content_hash = ContextDigest::from_bytes(bytes.as_ref());
        let admission = queue
            .try_admit(
                forward,
                u64::try_from(bytes.len()).expect("test body length"),
            )
            .expect("test analysis admission");
        let context_limits =
            ContextAnalysisLimits::new(tracepress_context::ContextAnalysisLimitValues {
                max_analyzed_bytes: 4_096,
                max_blocks: 64,
                max_json_depth: 64,
                max_string_bytes_inspected: 4_096,
                max_analysis_work_units: 100_000,
                max_analysis_wall_time_ms: 250,
                max_batches: 1,
            })
            .expect("context limits");
        DeferredAnalysisJob {
            forward,
            wire_body: WireBody::new(bytes),
            content_encoding: ContentEncoding::Identity,
            decode_limits: DecodeLimits {
                max_compressed_bytes: 4096,
                max_decompressed_bytes: 4096,
                max_expansion_ratio: None,
                max_decode_time: Duration::from_secs(1),
            },
            context_limits,
            wire_content_hash,
            sink,
            admission,
            enqueued_at: Instant::now(),
        }
    }

    #[tokio::test]
    async fn active_next_forward_defers_context_analysis_until_cpu_is_available() {
        let active_forwards = Arc::new(AtomicU64::new(0));
        let active_notify = Arc::new(Notify::new());
        let next_forward = ActiveForwardGuard::acquire(&active_forwards, &active_notify);
        let sink = Arc::new(TriggerSink::default());
        test_context_trigger(&active_forwards, &active_notify, Arc::clone(&sink)).start();
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(sink.results.lock().expect("test sink lock").is_empty());
        drop(next_forward);
        assert_eq!(
            wait_for_trigger_result(&sink).await,
            TriggerResult::Analyzed
        );
        assert_eq!(active_forwards.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn quiescent_context_analysis_completes() {
        let active_forwards = Arc::new(AtomicU64::new(0));
        let active_notify = Arc::new(Notify::new());
        let sink = Arc::new(TriggerSink::default());
        test_context_trigger(&active_forwards, &active_notify, Arc::clone(&sink)).start();
        assert_eq!(
            wait_for_trigger_result(&sink).await,
            TriggerResult::Analyzed
        );
        assert_eq!(active_forwards.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn deferred_analysis_is_fifo_by_forward_id() {
        let active_forwards = Arc::new(AtomicU64::new(0));
        let active_notify = Arc::new(Notify::new());
        let active = ActiveForwardGuard::acquire(&active_forwards, &active_notify);
        let queue = DeferredAnalysisQueue::new(
            BackgroundTracker::new(),
            Arc::clone(&active_forwards),
            Arc::clone(&active_notify),
        );
        let sink = Arc::new(OrderingSink::default());
        let first = deferred_job(&queue, ForwardId(1), Arc::clone(&sink));
        let second = deferred_job(&queue, ForwardId(2), Arc::clone(&sink));
        // Completion order is deliberately reversed; admission order remains authoritative.
        queue.enqueue(second);
        queue.enqueue(first);
        drop(active);

        for _ in 0..1_000 {
            if sink.forwards.lock().expect("ordering sink lock").len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(
            sink.forwards.lock().expect("ordering sink lock").as_slice(),
            &[ForwardId(1), ForwardId(2)]
        );
    }

    #[tokio::test]
    async fn deferred_analysis_processes_100_chained_jobs_without_capacity_drop() {
        let active_forwards = Arc::new(AtomicU64::new(0));
        let active_notify = Arc::new(Notify::new());
        let background = BackgroundTracker::new();
        let queue = DeferredAnalysisQueue::new(
            background.clone(),
            Arc::clone(&active_forwards),
            Arc::clone(&active_notify),
        );
        let sink = Arc::new(OrderingSink::default());

        for batch in 0..7 {
            let start = batch * 16;
            let end = ((batch + 1) * 16).min(100);
            for index in start..end {
                queue.enqueue(deferred_job(&queue, ForwardId(index), Arc::clone(&sink)));
            }
            // Keep the synthetic chain inside the explicit hard bound while still exercising
            // repeated admission, worker startup, FIFO processing, and queue reuse.
            background.wait().await;
        }

        let metrics = queue.metrics();
        assert_eq!(sink.forwards.lock().expect("ordering sink lock").len(), 100);
        assert_eq!(metrics.deferred_total, 100);
        assert_eq!(metrics.processed_deferred_total, 100);
        assert_eq!(metrics.backlog_capacity_drops, 0);
        assert_eq!(metrics.queue_items, 0);
        assert_eq!(metrics.queue_bytes, 0);
    }
}
