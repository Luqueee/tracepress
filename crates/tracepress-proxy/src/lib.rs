#![allow(
    clippy::multiple_crate_versions,
    reason = "the HTTP client and TLS stack currently require distinct transitive platform crates"
)]

//! Transparent OpenAI-compatible HTTP forwarding for Tracepress Phase 1.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Instant;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, StatusCode, Uri};
use axum::response::Response;
use axum::routing::post;
use futures_util::{StreamExt as _, stream};
use thiserror::Error;
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes};
use tracepress_provider::{
    MAX_RETAINED_USAGE_BYTES, ObservationInput, ObservationLimitValues, ObservationLimits,
    ObservationStatus, OpenAiResponsesV1Observer, ProviderEndpoint, ProviderObserver,
    RequestObservation, ResponseObservation, StreamingObserver,
};

const OBSERVATION_QUEUE_CAPACITY: usize = 32;

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

/// Synchronous, non-blocking boundary for Responses v1 semantic observations.
pub trait ProviderObservationSink: Send + Sync + 'static {
    /// Attempts to accept a parsed request observation of one forward.
    ///
    /// # Errors
    /// Returns a sink-local failure that the proxy deliberately ignores.
    fn try_record_request(
        &self,
        forward: ForwardId,
        observation: RequestObservation,
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
    fn try_record_request(
        &self,
        _forward: ForwardId,
        _observation: RequestObservation,
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

/// Validated transparent proxy configuration.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// Full upstream endpoint used as the authority and query base for both routes.
    pub upstream: ProviderEndpoint,
    /// Maximum accepted request body size.
    pub max_request_body_bytes: MaxRequestBodyBytes,
    /// Maximum streamed response body size.
    pub max_response_body_bytes: MaxResponseBodyBytes,
}

impl ProxyConfig {
    /// Creates a transparent proxy configuration from validated boundaries.
    #[must_use]
    pub const fn new(
        upstream: ProviderEndpoint,
        max_request_body_bytes: MaxRequestBodyBytes,
        max_response_body_bytes: MaxResponseBodyBytes,
    ) -> Self {
        Self {
            upstream,
            max_request_body_bytes,
            max_response_body_bytes,
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
    forwards: Arc<AtomicU64>,
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
            client,
            config,
            metadata: Arc::new(NoopMetadataSink),
            observations: Arc::new(NoopProviderObservationSink),
            forwards: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Allocates the correlation identity of the next accepted forward.
    fn next_forward(&self) -> ForwardId {
        ForwardId(self.forwards.fetch_add(1, Ordering::Relaxed))
    }

    /// Installs an allowlisted metadata sink. Sink behavior never gates forwarding.
    #[must_use]
    pub fn with_metadata_sink(mut self, metadata: Arc<dyn MetadataSink>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Installs a non-blocking Responses v1 semantic observation sink.
    #[must_use]
    pub fn with_observation_sink(mut self, sink: Arc<dyn ProviderObservationSink>) -> Self {
        self.observations = sink;
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

async fn forward_inner(
    proxy: &TransparentProxy,
    request: Request,
    route: InboundRoute,
) -> Result<Response, ForwardError> {
    let forward = proxy.next_forward();
    // Only the semantically observed route reports request, response and transport evidence.
    let observed = matches!(route, InboundRoute::Responses);
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
    // The request observer owns a refcounted handle to these same bytes and runs on a detached
    // blocking task: this task performs no semantic parsing and never waits for the observation.
    let stream_hint = observed
        .then(|| queue_request_observation(proxy, forward, &bytes))
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
            if observed {
                queue_transport_failure(proxy, forward, TransportFailure::classify(&error));
            }
            return Err(ForwardError::Upstream(error));
        }
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
            },
        ))
    } else {
        Body::from_stream(bounded_response_stream(
            upstream.bytes_stream(),
            maximum_response,
        ))
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok(response)
}

/// Forwards response bytes verbatim while enforcing the configured response bound.
fn bounded_response_stream<StreamType>(
    source: StreamType,
    maximum_response: u64,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>>
where
    StreamType: futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    source.scan((0_u64, false), move |(seen, finished), item| {
        let next = match item {
            Ok(chunk) if !*finished => match u64::try_from(chunk.len())
                .ok()
                .and_then(|length| seen.checked_add(length))
            {
                Some(total) if total <= maximum_response => {
                    *seen = total;
                    Some(Ok(chunk))
                }
                _ => {
                    *finished = true;
                    Some(Err(std::io::Error::other(
                        "response body exceeds configured limit",
                    )))
                }
            },
            Err(error) if !*finished => {
                *finished = true;
                Some(Err(std::io::Error::other(error)))
            }
            Ok(_) | Err(_) => None,
        };
        std::future::ready(next)
    })
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

enum ObservationMessage {
    Chunk(Bytes),
    Finished,
    Disconnected,
}

/// Hands the accepted request buffer to a detached observer and returns its stream hint.
///
/// Both the parse and the sink call run on a blocking task holding a refcounted handle to the same
/// bytes, so the forwarding task neither parses nor waits. The returned receiver resolves to the
/// requested `stream` flag, which only matters when the upstream declares no content type.
fn queue_request_observation(
    proxy: &TransparentProxy,
    forward: ForwardId,
    bytes: &Bytes,
) -> Option<tokio::sync::oneshot::Receiver<Option<bool>>> {
    let limits = observation_limits(proxy.config.max_request_body_bytes.get())?;
    let sink = Arc::clone(&proxy.observations);
    let bytes = bytes.clone();
    let (hint, receiver) = tokio::sync::oneshot::channel();
    drop(tokio::task::spawn_blocking(move || {
        let observation = OpenAiResponsesV1Observer::new()
            .observe_request(ObservationInput::new(&bytes, limits))
            .unwrap_or_else(|_error| {
                // The built-in observer currently returns a value for every bounded input.
                tracepress_provider::parse_request(ObservationInput::new(&bytes, limits))
            });
        // The hint is published before the sink runs, so a slow sink cannot delay the response
        // half's observer selection.
        let _delivered = hint.send(observation.stream);
        sink.try_record_request(forward, observation)
    }));
    Some(receiver)
}

/// Reports a forward that obtained no upstream response, without blocking the failing task.
fn queue_transport_failure(
    proxy: &TransparentProxy,
    forward: ForwardId,
    failure: TransportFailure,
) {
    let sink = Arc::clone(&proxy.observations);
    drop(tokio::task::spawn_blocking(move || {
        sink.try_record_transport_failure(forward, failure)
    }));
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
}

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
    let _response_observer_task = tokio::spawn(observe_response_task(ResponseObservationTask {
        receiver,
        forward: config.forward,
        sink: config.sink,
        selection: config.mode,
        limits,
        dropped: worker_dropped,
    }));
    let state = TapState {
        sender: Some(sender),
        dropped,
        maximum_response: config.maximum_response,
        upstream_length: config.upstream_length,
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
    seen: u64,
    settled: bool,
}

impl Drop for TapState {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        // A consumer stops polling a `Content-Length` body as soon as every advertised byte was
        // forwarded, so exhausting that length is a completed forward, not a transport abort.
        let message = if self.upstream_length == Some(self.seen) {
            ObservationMessage::Finished
        } else {
            ObservationMessage::Disconnected
        };
        queue_observation(&mut self.sender, &self.dropped, message);
        drop(self.sender.take());
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
    drop(tokio::task::spawn_blocking(move || {
        sink.try_record_response(forward, response)
    }));
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

#[cfg(test)]
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
}
