#![allow(
    clippy::multiple_crate_versions,
    reason = "the HTTP client and TLS stack currently require distinct transitive platform crates"
)]

//! Transparent OpenAI-compatible HTTP forwarding for Tracepress Phase 1.

use std::num::NonZeroUsize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, StatusCode, Uri};
use axum::response::Response;
use axum::routing::post;
use futures_util::{StreamExt as _, stream};
use thiserror::Error;
use tokio::sync::{Notify, Semaphore};
pub use tracepress_context::ContextAnalysisDropReason;
use tracepress_context::{
    ContextAnalysisLimits, ContextAnalysisLimitsError, ContextAnalysisResult, analyze_responses,
};
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes, ResourceLimits};
use tracepress_provider::{
    MAX_RETAINED_USAGE_BYTES, ObservationInput, ObservationLimitValues, ObservationLimits,
    ObservationStatus, OpenAiResponsesV1Observer, ProviderEndpoint, ProviderObserver,
    RequestObservation, ResponseObservation, StreamingObserver,
};

/// Gives a newly accepted N+1 forward priority over detached analysis from N.
///
/// The grace period is deliberately longer than one scheduler tick so an immediately following
/// request can publish its active-forward guard before analysis acquires CPU.
const CONTEXT_ANALYSIS_QUIESCENCE_WINDOW: Duration = Duration::from_millis(10);
const OBSERVATION_QUEUE_CAPACITY: usize = 32;

/// Hard cap on concurrent context analyses, even when the process advertises more capacity.
///
/// Each analysis retains a refcounted request prefix and up to the analyzer's bounded block
/// drafts, so allowing every runtime worker to analyze at once would turn burst width into
/// memory pressure. The effective value is the smaller of this cap, available parallelism, and
/// the configured CPU-work budget.
const CONTEXT_ANALYSIS_CONCURRENCY_HARD_CAP: usize = 2;

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
}

impl ActiveForwardGuard {
    fn acquire(active: &Arc<AtomicU64>) -> Self {
        let _ = active.fetch_add(1, Ordering::AcqRel);
        Self {
            active: Arc::clone(active),
        }
    }
}

impl Drop for ActiveForwardGuard {
    fn drop(&mut self) {
        let previous = self.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "active forward counter underflow");
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
}

impl InboundRoute {
    const fn path(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
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
    /// The analyzer produced the bounded result for the exact forwarded request bytes.
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
    /// Maximum number of context analyses allowed to run concurrently.
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

/// Cloneable Axum state for one transparent ingress.
#[derive(Clone)]
pub struct TransparentProxy {
    client: reqwest::Client,
    config: ProxyConfig,
    metadata: Arc<dyn MetadataSink>,
    observations: Arc<dyn ProviderObservationSink>,
    observations_enabled: bool,
    context_analysis_permits: Arc<Semaphore>,
    forwards: Arc<AtomicU64>,
    active_forwards: Arc<AtomicU64>,
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
            .build()
            .map_err(ForwardError::ClientBuild)?;
        Ok(Self {
            context_analysis_permits: Arc::new(Semaphore::new(
                config.context_analysis_concurrency.get(),
            )),
            client,
            config,
            metadata: Arc::new(NoopMetadataSink),
            observations: Arc::new(NoopProviderObservationSink),
            observations_enabled: false,
            forwards: Arc::new(AtomicU64::new(0)),
            active_forwards: Arc::new(AtomicU64::new(0)),
            background: BackgroundTracker::new(),
        })
    }

    /// Allocates the correlation identity of the next accepted forward.
    fn next_forward(&self) -> ForwardId {
        ForwardId(self.forwards.fetch_add(1, Ordering::Relaxed))
    }
    /// Marks one request as active until its response body or failure path settles.
    fn begin_forward(&self) -> ActiveForwardGuard {
        ActiveForwardGuard::acquire(&self.active_forwards)
    }
    /// Waits for detached provider/context observers before recorder shutdown.
    pub async fn wait_for_background_tasks(&self) {
        self.background.wait().await;
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
            .with_state(self)
    }
}

async fn forward_chat(State(proxy): State<TransparentProxy>, request: Request) -> Response {
    forward(&proxy, request, InboundRoute::ChatCompletions).await
}

async fn forward_responses(State(proxy): State<TransparentProxy>, request: Request) -> Response {
    forward(&proxy, request, InboundRoute::Responses).await
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
    let headers = forwarded_headers(&parts.headers);
    let target = match upstream_uri(&proxy.config.upstream, route) {
        Ok(target) => target,
        Err(error) => {
            if observed {
                queue_transport_failure(proxy, forward, TransportFailure::Endpoint);
            }
            return Err(error);
        }
    };
    // Phase 2 request parsing may begin as soon as the body is available. Phase 3 context analysis
    // is admitted only in shadow mode and after the upstream dispatch returns. This keeps the
    // provider header path free of analysis CPU while preserving the request-side correlation
    // boundary.
    let observation = observed
        .then(|| queue_request_observation(proxy, forward, bytes.clone()))
        .flatten();
    let upstream = match proxy
        .client
        .post(target)
        .headers(headers)
        .body(bytes)
        .send()
        .await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            if context_enabled {
                if let Some(observation) = observation {
                    queue_context_analysis(proxy, forward, observation)
                        .trigger
                        .start();
                }
            }
            if observed {
                queue_transport_failure(proxy, forward, TransportFailure::classify(&error));
            }
            return Err(ForwardError::Upstream(error));
        }
    };
    let (stream_hint, context_trigger) = if context_enabled {
        observation.map_or((None, None), |observation| {
            let queued = queue_context_analysis(proxy, forward, observation);
            (Some(queued.stream_hint), Some(queued.trigger))
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
    let body = if let Some(mode) = selection {
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
    observation: tokio::sync::oneshot::Receiver<RequestObservation>,
    bytes: Bytes,
}
fn queue_request_observation(
    proxy: &TransparentProxy,
    forward: ForwardId,
    bytes: Bytes,
) -> Option<QueuedRequestObservation> {
    let limits = observation_limits(proxy.config.max_request_body_bytes.get())?;
    let sink = Arc::clone(&proxy.observations);
    let (hint, stream_hint) = tokio::sync::oneshot::channel();
    let (sender, observation) = tokio::sync::oneshot::channel();
    let parse_bytes = bytes.clone();
    let task_guard = proxy.background.guard();
    drop(tokio::task::spawn_blocking(move || {
        let _task_guard = task_guard;
        let parsed = OpenAiResponsesV1Observer::new()
            .observe_request(ObservationInput::new(&parse_bytes, limits))
            .unwrap_or_else(|_error| {
                tracepress_provider::parse_request(ObservationInput::new(&parse_bytes, limits))
            });
        let _accepted = sink.try_record_request_context(RequestContextObservation {
            forward,
            observation: parsed.clone(),
            context: None,
        });
        let _delivered = hint.send(parsed.stream);
        let _sent = sender.send(parsed);
    }));
    Some(QueuedRequestObservation {
        stream_hint,
        observation,
        bytes,
    })
}

/// Defers Phase 3 analysis until the response body has settled.
struct QueuedContextAnalysis {
    stream_hint: tokio::sync::oneshot::Receiver<Option<bool>>,
    trigger: ContextAnalysisTrigger,
}

struct ContextAnalysisTrigger {
    forward: ForwardId,
    observation: tokio::sync::oneshot::Receiver<RequestObservation>,
    bytes: Bytes,
    context_limits: ContextAnalysisLimits,
    permits: Arc<Semaphore>,
    sink: Arc<dyn ProviderObservationSink>,
    active_forwards: Arc<AtomicU64>,
    background: BackgroundTracker,
}

fn try_context_analysis_permit(
    permits: &Arc<Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, ContextAnalysisDropReason> {
    Arc::clone(permits)
        .try_acquire_owned()
        .map_err(|_error| ContextAnalysisDropReason::ObserverBackpressure)
}

impl ContextAnalysisTrigger {
    fn start(self) {
        let Self {
            forward,
            observation,
            bytes,
            context_limits,
            permits,
            sink,
            active_forwards,
            background,
        } = self;
        let task_guard = background.guard();
        drop(tokio::spawn(async move {
            let _task_guard = task_guard;
            let Ok(_observation) = observation.await else {
                return;
            };
            tokio::time::sleep(CONTEXT_ANALYSIS_QUIESCENCE_WINDOW).await;
            let blocking_background = background.clone();
            let blocking_guard = blocking_background.guard();
            let _ = tokio::task::spawn_blocking(move || {
                let _task_guard = blocking_guard;
                let permit = match try_context_analysis_permit(&permits) {
                    Ok(permit) => permit,
                    Err(reason) => {
                        let _accepted =
                            sink.try_record_context_analysis(ContextAnalysisObservation {
                                forward,
                                outcome: ContextAnalysisOutcome::Dropped(reason),
                            });
                        return;
                    }
                };
                if active_forwards.load(Ordering::Acquire) != 0 {
                    let _accepted = sink.try_record_context_analysis(ContextAnalysisObservation {
                        forward,
                        outcome: ContextAnalysisOutcome::Dropped(
                            ContextAnalysisDropReason::ObserverBackpressure,
                        ),
                    });
                    return;
                }
                let analysis = analyze_responses(&bytes, context_limits);
                let _accepted = sink.try_record_context_analysis(ContextAnalysisObservation {
                    forward,
                    outcome: ContextAnalysisOutcome::Analyzed(analysis),
                });
                drop(permit);
            })
            .await;
        }));
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
        observation,
        bytes,
    } = queued;
    QueuedContextAnalysis {
        stream_hint,
        trigger: ContextAnalysisTrigger {
            forward,
            observation,
            bytes,
            context_limits: proxy.config.context_analysis_limits,
            permits: Arc::clone(&proxy.context_analysis_permits),
            sink: Arc::clone(&proxy.observations),
            active_forwards: Arc::clone(&proxy.active_forwards),
            background: proxy.background.clone(),
        },
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
    url.set_path(route.path());
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
    fn context_analysis_cap_plus_one_is_explicit_backpressure() {
        let permits = Arc::new(Semaphore::new(CONTEXT_ANALYSIS_CONCURRENCY_HARD_CAP));
        let first = try_context_analysis_permit(&permits);
        let second = try_context_analysis_permit(&permits);
        assert!(first.is_ok(), "first analysis should acquire its permit");
        assert!(second.is_ok(), "second analysis should acquire its permit");
        assert!(matches!(
            try_context_analysis_permit(&permits),
            Err(ContextAnalysisDropReason::ObserverBackpressure)
        ));
        drop((first, second));
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

    fn test_context_trigger(
        active_forwards: Arc<AtomicU64>,
        sink: Arc<TriggerSink>,
    ) -> ContextAnalysisTrigger {
        let bytes = Bytes::from_static(br#"{"model":"test","input":"value"}"#);
        let observation_limits = observation_limits(4_096).expect("observation limits");
        let parsed = OpenAiResponsesV1Observer::new()
            .observe_request(ObservationInput::new(&bytes, observation_limits))
            .unwrap_or_else(|_error| {
                tracepress_provider::parse_request(ObservationInput::new(
                    &bytes,
                    observation_limits,
                ))
            });
        let (sender, observation) = tokio::sync::oneshot::channel();
        sender.send(parsed).expect("observation receiver is live");
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
            observation,
            bytes,
            context_limits,
            permits: Arc::new(Semaphore::new(2)),
            sink,
            active_forwards,
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

    #[tokio::test]
    async fn active_next_forward_drops_context_analysis_before_cpu_work() {
        let active_forwards = Arc::new(AtomicU64::new(0));
        let next_forward = ActiveForwardGuard::acquire(&active_forwards);
        let sink = Arc::new(TriggerSink::default());
        test_context_trigger(Arc::clone(&active_forwards), Arc::clone(&sink)).start();
        assert_eq!(
            wait_for_trigger_result(&sink).await,
            TriggerResult::Dropped(ContextAnalysisDropReason::ObserverBackpressure)
        );
        drop(next_forward);
        assert_eq!(active_forwards.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn quiescent_context_analysis_completes() {
        let active_forwards = Arc::new(AtomicU64::new(0));
        let sink = Arc::new(TriggerSink::default());
        test_context_trigger(Arc::clone(&active_forwards), Arc::clone(&sink)).start();
        assert_eq!(
            wait_for_trigger_result(&sink).await,
            TriggerResult::Analyzed
        );
        assert_eq!(active_forwards.load(Ordering::Acquire), 0);
    }
}
