//! The semantic observer must never gate forwarding, and the lifecycle it reports must match
//! what actually happened on the wire.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::post;
use futures_util::{StreamExt as _, stream};
use tokio::net::{TcpListener, TcpStream};
use tracepress_core::{ResourceLimits, ResourceLimitsConfig};
use tracepress_provider::{
    ObservationStatus, ProviderEndpoint, ProviderResponseState, RequestObservation,
    ResponseObservation, UsageStatus,
};
use tracepress_proxy::{
    ContextAnalysisMode, ContextAnalysisObservation, ForwardId, ObservationSinkError,
    ProviderObservationSink, ProxyConfig, RequestContextObservation, TransparentProxy,
    TransportFailure,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn resource_limits(
    request_bytes: u64,
    response_bytes: u64,
) -> Result<ResourceLimits, Box<dyn std::error::Error>> {
    Ok(ResourceLimits::try_from(ResourceLimitsConfig {
        max_raw_bytes: Some(i128::from(request_bytes)),
        max_request_body_bytes: Some(i128::from(request_bytes)),
        max_response_body_bytes: Some(i128::from(response_bytes)),
        max_decompressed_bytes: Some(i128::from(response_bytes)),
        max_ipc_frame_bytes: Some(65_536),
        max_ipc_queue_items: Some(128),
        max_json_nesting: Some(64),
        max_json_items: Some(100_000),
        max_line_bytes: Some(i128::from(request_bytes)),
        max_processing_time_ms: Some(250),
        max_cpu_work_units: Some(1_000_000),
    })?)
}
/// Concurrent forwards used to expose semantic parsing left on the forwarding task.
const HOT_PATH_FORWARDS: usize = 12;
/// Nested groups per body, sized to saturate the parser's bounded item budget.
const HOT_PATH_GROUPS: usize = 25_000;
/// Upper bound on how long the last upstream dispatch may lag behind the client requests.
///
/// Measured on this workload: ~64 ms with the parse detached, ~513 ms when the same parse runs on
/// the forwarding task of a single-threaded runtime.
const DISPATCH_BUDGET: Duration = Duration::from_millis(250);
/// Blocking cost of one sink call, proving sink latency never reaches the forwarding task.
const SINK_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone)]
enum UpstreamBody {
    /// One buffered body advertised with an exact `Content-Length`.
    Sized(Bytes),
    /// A chunked body that never ends, so only the client can terminate the stream.
    Endless(Bytes),
}

#[derive(Clone)]
struct UpstreamState {
    status: StatusCode,
    content_type: Option<&'static str>,
    body: UpstreamBody,
    arrivals: Arc<Mutex<Vec<Arrival>>>,
}

#[derive(Clone, Debug)]
struct Arrival {
    at: Instant,
    body: Bytes,
}

struct ProxyTestConfig {
    upstream: ProviderEndpoint,
    request_limit: u64,
    response_limit: u64,
    sink: Arc<RecordingSink>,
}

impl ProxyTestConfig {
    /// Bounds that comfortably fit the small bodies of the lifecycle tests.
    const fn small(upstream: ProviderEndpoint, sink: Arc<RecordingSink>) -> Self {
        Self {
            upstream,
            request_limit: 4_096,
            response_limit: 4_096,
            sink,
        }
    }
}

#[derive(Clone, Copy)]
struct ExpectedCounts {
    requests: usize,
    responses: usize,
    failures: usize,
}

impl ExpectedCounts {
    const fn of(requests: usize, responses: usize, failures: usize) -> Self {
        Self {
            requests,
            responses,
            failures,
        }
    }
}

#[derive(Clone)]
struct RecordingSink {
    requests: Arc<Mutex<Vec<(ForwardId, RequestObservation)>>>,
    responses: Arc<Mutex<Vec<(ForwardId, ResponseObservation)>>>,
    failures: Arc<Mutex<Vec<(ForwardId, TransportFailure)>>>,
    request_delay: Duration,
}

impl RecordingSink {
    fn new(request_delay: Duration) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(Vec::new())),
            failures: Arc::new(Mutex::new(Vec::new())),
            request_delay,
        }
    }
}

impl Default for RecordingSink {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}

impl ProviderObservationSink for RecordingSink {
    fn try_record_request_context(
        &self,
        context: RequestContextObservation,
    ) -> Result<(), ObservationSinkError> {
        if self.request_delay > Duration::ZERO {
            std::thread::sleep(self.request_delay);
        }
        self.requests
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push((context.forward, context.observation));
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
        forward: ForwardId,
        observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        self.responses
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push((forward, observation));
        Ok(())
    }

    fn try_record_transport_failure(
        &self,
        forward: ForwardId,
        failure: TransportFailure,
    ) -> Result<(), ObservationSinkError> {
        self.failures
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push((forward, failure));
        Ok(())
    }
}

#[tokio::test]
async fn semantic_request_parsing_never_delays_upstream_dispatch() -> TestResult {
    // A body whose bounded semantic parse costs real CPU: run on the forwarding task of a
    // single-threaded runtime, every forward would queue behind that work.
    let nested = std::iter::repeat_n(r#"{"a":{"b":{"c":{"d":1}}}}"#, HOT_PATH_GROUPS)
        .collect::<Vec<_>>()
        .join(",");
    let request_body = format!(r#"{{"model":"hot","stream":false,"input":[{nested}]}}"#);
    let response_body = Bytes::from_static(br#"{"id":"resp_hot","status":"completed"}"#);
    let (upstream, arrivals, upstream_task) = spawn_upstream(UpstreamState {
        status: StatusCode::OK,
        content_type: Some("application/json"),
        body: UpstreamBody::Sized(response_body.clone()),
        arrivals: Arc::new(Mutex::new(Vec::new())),
    })
    .await?;
    let sink = Arc::new(RecordingSink::new(SINK_DELAY));
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 4 * 1024 * 1024,
        response_limit: 4 * 1024 * 1024,
        sink: Arc::<RecordingSink>::clone(&sink),
    })
    .await?;

    let client = reqwest::Client::new();
    let fired = Instant::now();
    let mut forwards = Vec::with_capacity(HOT_PATH_FORWARDS);
    for _forward in 0..HOT_PATH_FORWARDS {
        let client = client.clone();
        let url = format!("{proxy}/v1/responses");
        let body = request_body.clone();
        forwards.push(tokio::spawn(async move {
            let response = client
                .post(url)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await?;
            let status = response.status();
            let bytes = response.bytes().await?;
            Ok::<(StatusCode, Bytes), reqwest::Error>((status, bytes))
        }));
    }
    for forward in forwards {
        let (status, bytes) = forward.await??;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes.as_ref(), response_body.as_ref());
    }

    let arrivals = arrivals
        .lock()
        .map_err(|_error| "upstream arrivals poisoned")?
        .clone();
    assert_eq!(arrivals.len(), HOT_PATH_FORWARDS);
    let dispatch_lag = arrivals
        .iter()
        .map(|arrival| arrival.at.saturating_duration_since(fired))
        .max()
        .unwrap_or_default();
    for arrival in &arrivals {
        assert_eq!(
            arrival.body.as_ref(),
            request_body.as_bytes(),
            "forwarded request bytes must stay exact"
        );
    }
    let dispatch_lag_ms = dispatch_lag.as_millis();
    assert!(
        dispatch_lag < DISPATCH_BUDGET,
        "last upstream dispatch lagged {dispatch_lag_ms}ms, so semantic work is still on the forwarding task"
    );
    wait_for(&sink, ExpectedCounts::of(HOT_PATH_FORWARDS, 0, 0)).await?;

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn fully_forwarded_content_length_response_is_complete_not_disconnected() -> TestResult {
    let response_body = Bytes::from_static(
        br#"{"id":"resp_sized","model":"m","status":"completed","usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#,
    );
    let (upstream, _arrivals, upstream_task) = spawn_upstream(UpstreamState {
        status: StatusCode::OK,
        content_type: Some("application/json"),
        body: UpstreamBody::Sized(response_body.clone()),
        arrivals: Arc::new(Mutex::new(Vec::new())),
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig::small(
        upstream,
        Arc::<RecordingSink>::clone(&sink),
    ))
    .await?;

    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "application/json")
        .body(r#"{"model":"m","stream":false}"#)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_LENGTH).cloned(),
        Some(header::HeaderValue::from(response_body.len()))
    );
    assert_eq!(response.bytes().await?.as_ref(), response_body.as_ref());

    wait_for(&sink, ExpectedCounts::of(1, 1, 0)).await?;
    let observation = first_response(&sink)?;
    assert_eq!(
        observation.status,
        ObservationStatus::Complete,
        "a fully forwarded Content-Length body is not a partial observation"
    );
    assert_eq!(observation.response_state, ProviderResponseState::Completed);
    assert_eq!(observation.usage_status, UsageStatus::Final);
    assert!(observation.raw_usage.is_some());

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn aborted_stream_is_still_reported_as_a_disconnect() -> TestResult {
    let (upstream, _arrivals, upstream_task) = spawn_upstream(UpstreamState {
        status: StatusCode::OK,
        content_type: Some("text/event-stream"),
        body: UpstreamBody::Endless(Bytes::from_static(
            b"event: response.created\ndata: {\"response\":{\"id\":\"resp_open\"}}\n\n",
        )),
        arrivals: Arc::new(Mutex::new(Vec::new())),
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig::small(
        upstream,
        Arc::<RecordingSink>::clone(&sink),
    ))
    .await?;

    let read_task = tokio::spawn(async move {
        let response = reqwest::Client::new()
            .post(format!("{proxy}/v1/responses"))
            .header("content-type", "application/json")
            .body(r#"{"model":"m","stream":true}"#)
            .send()
            .await?;
        let mut body = response.bytes_stream();
        let first = body.next().await;
        Ok::<bool, reqwest::Error>(first.is_some())
    });
    let saw_first_event = read_task.await??;
    assert!(saw_first_event, "the client must receive streamed bytes");

    wait_for(&sink, ExpectedCounts::of(1, 1, 0)).await?;
    let observation = first_response(&sink)?;
    assert_eq!(
        observation.response_state,
        ProviderResponseState::Disconnected,
        "a stream abandoned before any terminal event is a disconnect"
    );
    assert_ne!(observation.status, ObservationStatus::Complete);

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn json_error_answering_a_streaming_request_is_observed_by_the_json_path() -> TestResult {
    let response_body = Bytes::from_static(
        br#"{"error":{"code":"rate_limit_exceeded","message":"slow down"},"type":"error"}"#,
    );
    let (upstream, _arrivals, upstream_task) = spawn_upstream(UpstreamState {
        status: StatusCode::TOO_MANY_REQUESTS,
        content_type: Some("application/json"),
        body: UpstreamBody::Sized(response_body.clone()),
        arrivals: Arc::new(Mutex::new(Vec::new())),
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig::small(
        upstream,
        Arc::<RecordingSink>::clone(&sink),
    ))
    .await?;

    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "application/json")
        .body(r#"{"model":"m","stream":true}"#)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.bytes().await?.as_ref(), response_body.as_ref());

    wait_for(&sink, ExpectedCounts::of(1, 1, 0)).await?;
    let observation = first_response(&sink)?;
    assert_eq!(
        observation.error_code.as_deref(),
        Some("rate_limit_exceeded"),
        "an application/json error body must reach the JSON observer, not the SSE framer"
    );
    assert_eq!(observation.status, ObservationStatus::Complete);
    assert_ne!(
        observation.response_state,
        ProviderResponseState::Disconnected
    );

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn absent_content_type_falls_back_to_the_request_stream_hint() -> TestResult {
    let streamed = observe_without_declared_content_type(
        true,
        b"event: response.completed\ndata: {\"response\":{\"id\":\"resp_hint_sse\",\"status\":\"completed\"}}\n\n",
    )
    .await?;
    assert_eq!(
        streamed.provider_response_id.as_deref(),
        Some("resp_hint_sse"),
        "a streaming hint must select the event framer when the upstream declared nothing"
    );
    assert_eq!(streamed.response_state, ProviderResponseState::Completed);

    let document = observe_without_declared_content_type(
        false,
        br#"{"id":"resp_hint_json","status":"completed"}"#,
    )
    .await?;
    assert_eq!(
        document.provider_response_id.as_deref(),
        Some("resp_hint_json"),
        "a non-streaming hint must select the document parser"
    );
    assert_eq!(document.response_state, ProviderResponseState::Completed);
    Ok(())
}

/// Forwards one body the upstream serves without declaring any content type.
async fn observe_without_declared_content_type(
    stream: bool,
    body: &'static [u8],
) -> Result<ResponseObservation, Box<dyn std::error::Error>> {
    let (upstream, _arrivals, upstream_task) = spawn_upstream(UpstreamState {
        status: StatusCode::OK,
        content_type: None,
        body: UpstreamBody::Sized(Bytes::from_static(body)),
        arrivals: Arc::new(Mutex::new(Vec::new())),
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig::small(
        upstream,
        Arc::<RecordingSink>::clone(&sink),
    ))
    .await?;
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "application/json")
        .body(format!(r#"{{"model":"m","stream":{stream}}}"#))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?.as_ref(), body);
    wait_for(&sink, ExpectedCounts::of(1, 1, 0)).await?;
    let observation = first_response(&sink)?;
    proxy_task.abort();
    upstream_task.abort();
    Ok(observation)
}

#[tokio::test]
async fn upstream_transport_failure_reaches_the_sink_and_still_answers_502() -> TestResult {
    // A listener that accepts the connection and closes it without answering.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            drop(stream);
        }
    });
    let upstream = ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig::small(
        upstream,
        Arc::<RecordingSink>::clone(&sink),
    ))
    .await?;

    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "application/json")
        .body(r#"{"model":"m","stream":false}"#)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        response.bytes().await?.as_ref(),
        b"upstream transport failed"
    );

    wait_for(&sink, ExpectedCounts::of(1, 0, 1)).await?;
    let failures = sink
        .failures
        .lock()
        .map_err(|_error| "failure sink poisoned")?
        .clone();
    let Some((forward, failure)) = failures.first().copied() else {
        return Err("transport failure was not recorded".into());
    };
    let requests = sink
        .requests
        .lock()
        .map_err(|_error| "request sink poisoned")?
        .clone();
    assert_eq!(
        requests.first().map(|(forward, _observation)| *forward),
        Some(forward),
        "the transport failure must carry the forward identity of the request observation"
    );
    assert!(
        ["connect", "timeout", "request", "endpoint", "other"].contains(&failure.label()),
        "the sink must receive a content-free failure label"
    );

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

async fn upstream_handler(State(state): State<UpstreamState>, request: Request) -> Response {
    let at = Instant::now();
    let (_parts, body) = request.into_parts();
    let body = to_bytes(body, 8 * 1024 * 1024).await.unwrap_or_default();
    if let Ok(mut arrivals) = state.arrivals.lock() {
        arrivals.push(Arrival { at, body });
    }
    let mut builder = Response::builder().status(state.status);
    if let Some(content_type) = state.content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    match state.body {
        UpstreamBody::Sized(bytes) => builder
            .header(header::CONTENT_LENGTH, bytes.len())
            .body(Body::from(bytes)),
        UpstreamBody::Endless(bytes) => builder.body(Body::from_stream(
            stream::iter([Ok::<Bytes, std::convert::Infallible>(bytes)]).chain(stream::pending()),
        )),
    }
    .unwrap_or_else(|_error| Response::new(Body::empty()))
}

async fn spawn_upstream(
    state: UpstreamState,
) -> Result<
    (
        ProviderEndpoint,
        Arc<Mutex<Vec<Arrival>>>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let arrivals = Arc::clone(&state.arrivals);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new()
        .route("/v1/responses", post(upstream_handler))
        .with_state(state);
    let task = tokio::spawn(async move {
        let _result = axum::serve(listener, router).await;
    });
    wait_until_listening(address).await?;
    Ok((
        ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?,
        arrivals,
        task,
    ))
}

async fn spawn_proxy(
    config: ProxyTestConfig,
) -> Result<(String, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
    let ProxyTestConfig {
        upstream,
        request_limit,
        response_limit,
        sink,
    } = config;
    let proxy = TransparentProxy::new(ProxyConfig::new(
        upstream,
        resource_limits(request_limit, response_limit)?,
        ContextAnalysisMode::Shadow,
    )?)?
    .with_observation_sink(sink);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _result = axum::serve(listener, proxy.router()).await;
    });
    wait_until_listening(address).await?;
    Ok((format!("http://{address}"), task))
}

fn first_response(sink: &RecordingSink) -> Result<ResponseObservation, Box<dyn std::error::Error>> {
    sink.responses
        .lock()
        .map_err(|_error| -> Box<dyn std::error::Error> { "response sink poisoned".into() })?
        .first()
        .map(|(_forward, observation)| observation.clone())
        .ok_or_else(|| "response sink had no observations".into())
}

async fn wait_for(sink: &RecordingSink, expected: ExpectedCounts) -> TestResult {
    let ExpectedCounts {
        requests,
        responses,
        failures,
    } = expected;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let observed = (
                sink.requests
                    .lock()
                    .map_err(|_error| "request sink poisoned")?
                    .len(),
                sink.responses
                    .lock()
                    .map_err(|_error| "response sink poisoned")?
                    .len(),
                sink.failures
                    .lock()
                    .map_err(|_error| "failure sink poisoned")?
                    .len(),
            );
            if observed.0 >= requests && observed.1 >= responses && observed.2 >= failures {
                break Ok::<(), Box<dyn std::error::Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    Ok(())
}

async fn wait_until_listening(address: std::net::SocketAddr) -> Result<(), std::io::Error> {
    for _attempt in 0..100 {
        match TcpStream::connect(address).await {
            Ok(stream) => {
                drop(stream);
                return Ok(());
            }
            Err(_error) => tokio::task::yield_now().await,
        }
    }
    Err(std::io::Error::other("server did not accept connections"))
}
