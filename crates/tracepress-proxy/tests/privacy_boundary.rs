//! End-to-end privacy and fail-open checks for the Responses ingress.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use futures_util::{StreamExt, stream};
use tokio::net::{TcpListener, TcpStream};
use tracepress_core::{ResourceLimits, ResourceLimitsConfig};
use tracepress_provider::{
    ObservationStatus, ProviderEndpoint, ProviderResponseState, RequestObservation,
    ResponseObservation, UsageStatus,
};
use tracepress_proxy::{
    ContextAnalysisMode, ContextAnalysisObservation, ForwardId, ForwardMetadata, InboundRoute,
    MetadataSink, MetadataSinkError, ObservationSinkError, ProviderObservationSink, ProxyConfig,
    RequestContextObservation, TransparentProxy, TransportFailure,
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

const AUTH_CANARY: &str = "auth-canary-7f3a";
const COOKIE_CANARY: &str = "cookie-canary-1c2b";
const KEY_CANARY: &str = "api-key-canary-9d4e";
const PASSWORD_CANARY: &str = "password-canary-4a6e";
const PROMPT_CANARY: &str = "prompt-canary-5b8f";
const INSTRUCTIONS_CANARY: &str = "instructions-canary-3e91";
const METADATA_CANARY: &str = "metadata-canary-6c20";
const USER_CANARY: &str = "user-canary-8d13";
const CACHE_CANARY: &str = "cache-canary-2f74";
const SAFETY_CANARY: &str = "safety-canary-0a65";
const TOOL_ARGUMENT_CANARY: &str = "tool-argument-canary-9b31";
const OUTPUT_CANARY: &str = "output-canary-7e42";

#[derive(Clone, Debug)]
struct UpstreamState {
    status: StatusCode,
    content_type: &'static str,
    chunks: Vec<Bytes>,
    observed: Arc<Mutex<Option<ObservedRequest>>>,
    never_end: bool,
}

#[derive(Debug)]
struct UpstreamConfig {
    status: StatusCode,
    content_type: &'static str,
    chunks: Vec<Bytes>,
    never_end: bool,
}

struct ProxyTestConfig {
    upstream: ProviderEndpoint,
    request_limit: u64,
    response_limit: u64,
    sink: Arc<RecordingSink>,
}

#[derive(Clone, Debug)]
struct ObservedRequest {
    body: Bytes,
    headers: HeaderMap,
    path: String,
}

#[derive(Clone, Default)]
struct RecordingSink {
    metadata: Arc<Mutex<Vec<ForwardMetadata>>>,
    requests: Arc<Mutex<Vec<(ForwardId, RequestObservation)>>>,
    responses: Arc<Mutex<Vec<(ForwardId, ResponseObservation)>>>,
    failures: Arc<Mutex<Vec<(ForwardId, TransportFailure)>>>,
}

impl MetadataSink for RecordingSink {
    fn try_record(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        self.metadata
            .lock()
            .map_err(|_error| MetadataSinkError::rejected())?
            .push(metadata);
        Ok(())
    }
}

impl ProviderObservationSink for RecordingSink {
    fn try_record_request_context(
        &self,
        context: RequestContextObservation,
    ) -> Result<(), ObservationSinkError> {
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
async fn responses_forward_canaries_exactly_but_observe_only_allowlisted_metadata() -> TestResult {
    let request_body = format!(
        r#"{{"model":"safe-model","input":[{{"role":"user","content":[{{"type":"input_text","text":"{PROMPT_CANARY}"}}]}} ,{{"type":"function_call","name":"safe_tool","arguments":"{TOOL_ARGUMENT_CANARY}"}}],"tools":[{{"type":"function","name":"safe_tool"}}],"instructions":"{INSTRUCTIONS_CANARY}","metadata":{{"tag":"{METADATA_CANARY}"}},"user":"{USER_CANARY}","prompt_cache_key":"{CACHE_CANARY}","safety_identifier":"{SAFETY_CANARY}","api_key":"{KEY_CANARY}","password":"{PASSWORD_CANARY}","stream":false}}"#
    );
    let response_body = format!(
        r#"{{"id":"safe-response","model":"safe-model","status":"completed","output":[{{"type":"message","content":[{{"type":"output_text","text":"{OUTPUT_CANARY}"}}]}}],"metadata":{{"tag":"{METADATA_CANARY}"}},"usage":{{"input_tokens":3,"output_tokens":2,"total_tokens":5}}}}"#
    );
    let (upstream, observed, upstream_task) = spawn_upstream(UpstreamConfig {
        status: StatusCode::OK,
        content_type: "application/json",
        chunks: vec![Bytes::from(response_body.clone())],
        never_end: false,
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 32 * 1024,
        response_limit: 32 * 1024,
        sink: Arc::clone(&sink),
    })
    .await?;

    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("authorization", format!("Bearer {AUTH_CANARY}"))
        .header("cookie", COOKIE_CANARY)
        .header("x-api-key", KEY_CANARY)
        .header("x-password", PASSWORD_CANARY)
        .header("content-type", "application/json")
        .body(request_body.clone())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?.as_ref(), response_body.as_bytes());

    wait_for_counts(&sink, 1, 1).await?;
    let observed_request = observed_request(&observed)?;
    assert_eq!(observed_request.path, "/v1/responses");
    assert_eq!(observed_request.body.as_ref(), request_body.as_bytes());
    assert_eq!(
        header(&observed_request.headers, "authorization"),
        Some(format!("Bearer {AUTH_CANARY}"))
    );
    assert_eq!(
        header(&observed_request.headers, "cookie"),
        Some(COOKIE_CANARY.to_owned())
    );
    assert_eq!(
        header(&observed_request.headers, "x-api-key"),
        Some(KEY_CANARY.to_owned())
    );
    assert_eq!(
        header(&observed_request.headers, "x-password"),
        Some(PASSWORD_CANARY.to_owned())
    );

    assert_canaries_not_observed(&sink)?;
    assert_metadata(&sink, u64::try_from(request_body.len())?)?;

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn malformed_oversized_and_mismatched_content_are_bounded_without_secret_observation()
-> TestResult {
    assert_malformed_request_is_bounded().await?;
    assert_malformed_response_is_bounded().await?;
    assert_mismatched_content_is_not_observed().await?;
    assert_oversized_request_is_rejected().await
}

async fn assert_malformed_request_is_bounded() -> TestResult {
    let malformed = format!(r#"{{"input":[{{"text":"{PROMPT_CANARY}"}}]"#);
    let valid_json_response = br#"{"status":"completed"}"#;
    let (upstream, observed, upstream_task) = spawn_upstream(UpstreamConfig {
        status: StatusCode::OK,
        content_type: "application/json",
        chunks: vec![Bytes::from_static(valid_json_response)],
        never_end: false,
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 4_096,
        response_limit: 4_096,
        sink: Arc::clone(&sink),
    })
    .await?;
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "application/json")
        .body(malformed)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?.as_ref(), valid_json_response);
    wait_for_counts(&sink, 1, 1).await?;
    assert_eq!(first_request(&sink)?.status, ObservationStatus::Malformed);
    let observation = first_response(&sink)?;
    assert_eq!(observation.status, ObservationStatus::Complete);
    assert_eq!(observation.usage_status, UsageStatus::Unavailable);
    assert!(observation.raw_usage.is_none());
    assert!(observation.normalized_usage.is_none());
    assert!(
        !format!(
            "{:?}",
            sink.requests
                .lock()
                .map_err(|_| "request sink poisoned")?
                .as_slice()
        )
        .contains(PROMPT_CANARY)
    );
    assert!(!observed_request(&observed)?.body.is_empty());
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

async fn assert_malformed_response_is_bounded() -> TestResult {
    let malformed_json_response = b"{not-json";
    let (upstream, _observed, upstream_task) = spawn_upstream(UpstreamConfig {
        status: StatusCode::OK,
        content_type: "application/json",
        chunks: vec![Bytes::from_static(malformed_json_response)],
        never_end: false,
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 4_096,
        response_limit: 4_096,
        sink: Arc::clone(&sink),
    })
    .await?;
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "application/json")
        .body(br#"{"model":"safe"}"#.as_slice())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?.as_ref(), malformed_json_response);
    wait_for_counts(&sink, 1, 1).await?;
    assert_eq!(first_response(&sink)?.status, ObservationStatus::Malformed);
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

async fn assert_mismatched_content_is_not_observed() -> TestResult {
    let mismatched_body = format!(r#"{{"model":"safe","input":"{PROMPT_CANARY}"}}"#);
    let (upstream, observed, upstream_task) = spawn_upstream(UpstreamConfig {
        status: StatusCode::OK,
        content_type: "text/plain",
        chunks: vec![Bytes::from_static(b"opaque-response")],
        never_end: false,
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 4_096,
        response_limit: 4_096,
        sink: Arc::clone(&sink),
    })
    .await?;
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "text/plain")
        .body(mismatched_body.clone())
        .send()
        .await?;
    assert_eq!(response.bytes().await?.as_ref(), b"opaque-response");
    wait_for_counts(&sink, 1, 0).await?;
    assert_eq!(
        observed_request(&observed)?.body.as_ref(),
        mismatched_body.as_bytes()
    );
    assert!(
        sink.responses
            .lock()
            .map_err(|_| "response sink poisoned")?
            .is_empty()
    );
    assert!(
        !format!(
            "{:?}",
            sink.requests
                .lock()
                .map_err(|_| "request sink poisoned")?
                .as_slice()
        )
        .contains(PROMPT_CANARY)
    );
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

async fn assert_oversized_request_is_rejected() -> TestResult {
    let oversized = format!(r#"{{"input":"{PROMPT_CANARY}"}}{}"#, "x".repeat(256));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/responses",
            post(|_request: Request| async {
                (StatusCode::OK, Bytes::from_static(b"must-not-forward"))
            }),
        );
        let _ = axum::serve(listener, app).await;
    });
    let upstream = ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 64,
        response_limit: 4_096,
        sink: Arc::clone(&sink),
    })
    .await?;
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .body(oversized)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!String::from_utf8_lossy(&response.bytes().await?).contains(PROMPT_CANARY));
    assert!(
        sink.requests
            .lock()
            .map_err(|_| "request sink poisoned")?
            .is_empty()
    );
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn nonterminal_sse_oversize_queue_pressure_and_disconnect_never_claim_completion()
-> TestResult {
    let canary_event =
        format!("event: response.output_text.delta\ndata: {{\"delta\":\"{OUTPUT_CANARY}\"}}\n\n");
    let mut chunks = vec![Bytes::from_static(b"event: response.created\ndata: {}\n\n")];
    chunks.extend((0..128).map(|_| Bytes::from(canary_event.clone())));
    let expected = chunks
        .iter()
        .flat_map(|chunk| chunk.iter().copied())
        .collect::<Vec<_>>();
    let (upstream, _observed, upstream_task) = spawn_upstream(UpstreamConfig {
        status: StatusCode::OK,
        content_type: "text/event-stream",
        chunks,
        never_end: false,
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 4_096,
        response_limit: 64 * 1024,
        sink: Arc::clone(&sink),
    })
    .await?;
    let request = br#"{"model":"safe","stream":true}"#;
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/responses"))
        .header("content-type", "application/json")
        .body(request.as_slice())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?.as_ref(), expected.as_slice());
    wait_for_counts(&sink, 1, 1).await?;
    let observation = first_response(&sink)?;
    assert_ne!(observation.status, ObservationStatus::Complete);
    assert_ne!(observation.response_state, ProviderResponseState::Completed);
    assert_eq!(observation.usage_status, UsageStatus::Unavailable);
    assert!(!format!("{observation:?}").contains(OUTPUT_CANARY));
    proxy_task.abort();
    upstream_task.abort();

    let (upstream, _observed, upstream_task) = spawn_upstream(UpstreamConfig {
        status: StatusCode::OK,
        content_type: "text/event-stream",
        chunks: vec![Bytes::from_static(b"event: response.created\ndata: {}\n\n")],
        never_end: true,
    })
    .await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) = spawn_proxy(ProxyTestConfig {
        upstream,
        request_limit: 4_096,
        response_limit: 4_096,
        sink: Arc::clone(&sink),
    })
    .await?;
    let read_task = tokio::spawn(async move {
        if let Ok(response) = reqwest::Client::new()
            .post(format!("{proxy}/v1/responses"))
            .header("content-type", "application/json")
            .body(br#"{"model":"safe","stream":true}"#.as_slice())
            .send()
            .await
        {
            let _ = response.bytes().await;
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    read_task.abort();
    wait_for_counts(&sink, 1, 1).await?;
    let observation = first_response(&sink)?;
    assert_ne!(observation.response_state, ProviderResponseState::Completed);
    assert_ne!(observation.status, ObservationStatus::Complete);
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

async fn upstream_handler(State(state): State<UpstreamState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 2 * 1024 * 1024).await.unwrap_or_default();
    if let Ok(mut observed) = state.observed.lock() {
        *observed = Some(ObservedRequest {
            body,
            headers: parts.headers,
            path: parts.uri.path().to_owned(),
        });
    }
    let chunks = stream::iter(
        state
            .chunks
            .into_iter()
            .map(Ok::<Bytes, std::convert::Infallible>),
    );
    let body = if state.never_end {
        Body::from_stream(chunks.chain(stream::pending()))
    } else {
        Body::from_stream(chunks)
    };
    Response::builder()
        .status(state.status)
        .header("content-type", state.content_type)
        .body(body)
        .unwrap_or_else(|_error| Response::new(Body::empty()))
}

async fn spawn_upstream(
    config: UpstreamConfig,
) -> Result<
    (
        ProviderEndpoint,
        Arc<Mutex<Option<ObservedRequest>>>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let UpstreamConfig {
        status,
        content_type,
        chunks,
        never_end,
    } = config;
    let observed = Arc::new(Mutex::new(None));
    let state = UpstreamState {
        status,
        content_type,
        chunks,
        observed: Arc::clone(&observed),
        never_end,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new()
        .route("/v1/responses", post(upstream_handler))
        .with_state(state);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    wait_until_listening(address).await?;
    Ok((
        ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?,
        observed,
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
    .with_metadata_sink(Arc::<RecordingSink>::clone(&sink))
    .with_observation_sink(Arc::<RecordingSink>::clone(&sink));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, proxy.router()).await;
    });
    wait_until_listening(address).await?;
    Ok((format!("http://{address}"), task))
}

fn first_request(sink: &RecordingSink) -> Result<RequestObservation, Box<dyn std::error::Error>> {
    sink.requests
        .lock()
        .map_err(|_| -> Box<dyn std::error::Error> { "request sink poisoned".into() })?
        .first()
        .map(|(_forward, observation)| observation.clone())
        .ok_or_else(|| "request sink had no observations".into())
}

fn first_response(sink: &RecordingSink) -> Result<ResponseObservation, Box<dyn std::error::Error>> {
    sink.responses
        .lock()
        .map_err(|_| -> Box<dyn std::error::Error> { "response sink poisoned".into() })?
        .first()
        .map(|(_forward, observation)| observation.clone())
        .ok_or_else(|| "response sink had no observations".into())
}

fn assert_metadata(sink: &RecordingSink, request_bytes: u64) -> TestResult {
    let records = sink
        .metadata
        .lock()
        .map_err(|_| -> Box<dyn std::error::Error> { "metadata sink poisoned".into() })?;
    assert_eq!(records.len(), 1);
    let Some(metadata) = records.first().copied() else {
        return Err("metadata sink had no observations".into());
    };
    drop(records);
    assert_eq!(metadata.request_bytes, request_bytes);
    assert_eq!(metadata.status_code, Some(200));
    assert_eq!(metadata.response_bytes, None);
    assert_eq!(metadata.route, InboundRoute::Responses);
    // Both semantic halves of this forward carry the transport record's correlation identity.
    assert_eq!(forward_ids(&sink.requests)?, vec![metadata.forward]);
    assert_eq!(forward_ids(&sink.responses)?, vec![metadata.forward]);
    Ok(())
}

fn forward_ids<ObservationType>(
    observations: &Arc<Mutex<Vec<(ForwardId, ObservationType)>>>,
) -> Result<Vec<ForwardId>, Box<dyn std::error::Error>> {
    Ok(observations
        .lock()
        .map_err(|_| -> Box<dyn std::error::Error> { "observation sink poisoned".into() })?
        .iter()
        .map(|(forward, _observation)| *forward)
        .collect())
}

fn assert_canaries_not_observed(sink: &RecordingSink) -> TestResult {
    let request_debug = format!(
        "{:?}",
        sink.requests
            .lock()
            .map_err(|_| "request sink poisoned")?
            .as_slice()
    );
    let response_debug = format!(
        "{:?}",
        sink.responses
            .lock()
            .map_err(|_| "response sink poisoned")?
            .as_slice()
    );
    let metadata_debug = format!(
        "{:?}",
        sink.metadata
            .lock()
            .map_err(|_| "metadata sink poisoned")?
            .as_slice()
    );
    let failure_debug = format!(
        "{:?}",
        sink.failures
            .lock()
            .map_err(|_| "transport failure sink poisoned")?
            .as_slice()
    );
    for canary in [
        AUTH_CANARY,
        COOKIE_CANARY,
        KEY_CANARY,
        PASSWORD_CANARY,
        PROMPT_CANARY,
        INSTRUCTIONS_CANARY,
        METADATA_CANARY,
        USER_CANARY,
        CACHE_CANARY,
        SAFETY_CANARY,
        TOOL_ARGUMENT_CANARY,
        OUTPUT_CANARY,
    ] {
        assert!(
            !request_debug.contains(canary),
            "request observation leaked {canary}"
        );
        assert!(
            !response_debug.contains(canary),
            "response observation leaked {canary}"
        );
        assert!(!metadata_debug.contains(canary), "metadata leaked {canary}");
        assert!(
            !failure_debug.contains(canary),
            "transport failure leaked {canary}"
        );
    }
    Ok(())
}

fn observed_request(
    observed: &Arc<Mutex<Option<ObservedRequest>>>,
) -> Result<ObservedRequest, Box<dyn std::error::Error>> {
    observed
        .lock()
        .map_err(|_| -> Box<dyn std::error::Error> { "upstream observation poisoned".into() })?
        .clone()
        .ok_or_else(|| "upstream did not observe request".into())
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn wait_for_counts(
    sink: &RecordingSink,
    request_count: usize,
    response_count: usize,
) -> TestResult {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let requests = sink
                .requests
                .lock()
                .map_err(|_| "request sink poisoned")?
                .len();
            let responses = sink
                .responses
                .lock()
                .map_err(|_| "response sink poisoned")?
                .len();
            if requests >= request_count && responses >= response_count {
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
