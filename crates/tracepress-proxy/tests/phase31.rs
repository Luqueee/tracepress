//! Phase 3.1 wire/analysis compatibility coverage.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::Request,
    http::StatusCode,
    response::Response,
    routing::post,
};
use tokio::net::TcpListener;
use tracepress_core::{ResourceLimits, ResourceLimitsConfig};
use tracepress_provider::{
    AnalysisDecodeStatus, ContentEncoding, ObservationStatus, ProviderEndpoint, RequestObservation,
    ResponseObservation,
};
use tracepress_proxy::{
    CompactionObservation, ContextAnalysisMode, ContextAnalysisObservation, ContextAnalysisOutcome,
    ForwardId, ObservationSinkError, ProviderObservationSink, ProxyConfig,
    RequestContextObservation, TransparentProxy, TransportFailure,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Debug)]
struct Arrival {
    path: String,
    body: Bytes,
    authorization: Option<String>,
    content_encoding: Option<String>,
    content_length: Option<String>,
}

#[derive(Clone, Default)]
struct Sink {
    requests: Arc<Mutex<Vec<RequestObservation>>>,
    contexts: Arc<Mutex<Vec<ContextAnalysisOutcome>>>,
    compactions: Arc<Mutex<Vec<CompactionObservation>>>,
}

impl ProviderObservationSink for Sink {
    fn try_record_request_context(
        &self,
        context: RequestContextObservation,
    ) -> Result<(), ObservationSinkError> {
        self.requests
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push(context.observation);
        Ok(())
    }

    fn try_record_context_analysis(
        &self,
        observation: ContextAnalysisObservation,
    ) -> Result<(), ObservationSinkError> {
        self.contexts
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push(observation.outcome);
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

    fn try_record_compaction(
        &self,
        observation: CompactionObservation,
    ) -> Result<(), ObservationSinkError> {
        self.compactions
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push(observation);
        Ok(())
    }
}

fn limits() -> Result<ResourceLimits, tracepress_core::ResourceLimitsError> {
    ResourceLimits::try_from(ResourceLimitsConfig {
        max_raw_bytes: Some(8 * 1024 * 1024),
        max_request_body_bytes: Some(8 * 1024 * 1024),
        max_response_body_bytes: Some(4 * 1024),
        max_decompressed_bytes: Some(32 * 1024 * 1024),
        max_ipc_frame_bytes: Some(65_536),
        max_ipc_queue_items: Some(128),
        max_json_nesting: Some(64),
        max_json_items: Some(100_000),
        max_line_bytes: Some(8 * 1024 * 1024),
        max_processing_time_ms: Some(250),
        max_cpu_work_units: Some(1_000_000),
    })
}

async fn wait_for(sink: &Sink) -> TestResult {
    for _attempt in 0..500 {
        let requests = sink
            .requests
            .lock()
            .map_err(|_error| "request lock poisoned")?
            .len();
        let contexts = sink
            .contexts
            .lock()
            .map_err(|_error| "context lock poisoned")?
            .len();
        if requests == 1 && contexts == 1 {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    Err("phase 3.1 observation did not settle".into())
}

async fn wait_for_compaction(sink: &Sink) -> TestResult {
    for _attempt in 0..500 {
        if !sink
            .compactions
            .lock()
            .map_err(|_error| "compaction lock poisoned")?
            .is_empty()
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    Err("compaction observation did not settle".into())
}

#[allow(
    clippy::too_many_lines,
    reason = "the test keeps the complete wire and analysis assertions in one vertical slice"
)]
#[tokio::test]
async fn zstd_forwarding_is_wire_exact_while_analysis_uses_json() -> TestResult {
    let arrivals = Arc::new(Mutex::new(Vec::<Arrival>::new()));
    let upstream_arrivals = Arc::clone(&arrivals);
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_address = upstream_listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/responses",
            post(move |request: Request| {
                let upstream_arrivals = Arc::clone(&upstream_arrivals);
                async move {
                    let (parts, body) = request.into_parts();
                    let body = to_bytes(body, 8 * 1024 * 1024).await.unwrap_or_default();
                    if let Ok(mut arrivals) = upstream_arrivals.lock() {
                        arrivals.push(Arrival {
                            path: parts.uri.path().to_owned(),
                            body,
                            authorization: parts
                                .headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            content_encoding: parts
                                .headers
                                .get("content-encoding")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            content_length: parts
                                .headers
                                .get("content-length")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                        });
                    }
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"id":"resp_test","status":"completed"}"#))
                        .unwrap_or_else(|_error| Response::new(Body::empty()))
                }
            }),
        );
        let _result = axum::serve(upstream_listener, app).await;
    });

    let sink = Arc::new(Sink::default());
    let sink_for_proxy: Arc<dyn ProviderObservationSink> = Arc::<Sink>::clone(&sink);
    let proxy = TransparentProxy::new(ProxyConfig::new(
        ProviderEndpoint::new(&format!("http://{upstream_address}/v1/responses"))?,
        limits()?,
        ContextAnalysisMode::Shadow,
    )?)?
    .with_observation_sink(sink_for_proxy);
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _result = axum::serve(proxy_listener, proxy.router()).await;
    });

    let json = br#"{"model":"gpt-5","input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"stream":false}"#;
    let wire = zstd::stream::encode_all(std::io::Cursor::new(json), 1)?;
    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/responses"))
        .header("content-type", "application/json")
        .header("content-encoding", "zstd")
        .header(
            "authorization",
            "Bearer TRACEPRESS_SUBSCRIPTION_SECRET_CANARY",
        )
        .body(wire.clone())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    wait_for(&sink).await?;

    let arrival = arrivals
        .lock()
        .map_err(|_error| "upstream lock poisoned")?
        .first()
        .cloned()
        .ok_or("upstream did not receive request")?;
    assert_eq!(arrival.path, "/v1/responses");
    assert_eq!(arrival.body.as_ref(), wire.as_slice());
    assert_eq!(arrival.content_encoding.as_deref(), Some("zstd"));
    assert_eq!(
        arrival.content_length.as_deref(),
        Some(wire.len().to_string().as_str())
    );
    assert_eq!(
        arrival.authorization.as_deref(),
        Some("Bearer TRACEPRESS_SUBSCRIPTION_SECRET_CANARY")
    );

    let observation = sink
        .requests
        .lock()
        .map_err(|_error| "request lock poisoned")?
        .first()
        .cloned()
        .ok_or("request observation missing")?;
    assert_eq!(observation.status, ObservationStatus::Complete);
    assert_eq!(observation.content_encoding, ContentEncoding::Zstd);
    assert_eq!(
        observation.analysis_decode_status,
        AnalysisDecodeStatus::Decoded
    );
    assert_eq!(observation.wire_bytes, Some(wire.len() as u64));
    assert_eq!(observation.request_bytes, Some(json.len() as u64));
    assert_eq!(observation.decoded_bytes, Some(json.len() as u64));
    assert_eq!(
        observation.wire_sha256.as_deref().map(<[u8]>::len),
        Some(32)
    );
    assert_eq!(observation.model.as_deref(), Some("gpt-5"));
    let context = sink
        .contexts
        .lock()
        .map_err(|_error| "context lock poisoned")?
        .pop()
        .ok_or("context analysis missing")?;
    let ContextAnalysisOutcome::Analyzed(context) = context else {
        return Err("valid zstd payload did not produce context analysis".into());
    };
    let wire_sha256 = observation
        .wire_sha256
        .as_deref()
        .ok_or("wire digest missing")?;
    assert_eq!(
        context.request_content_hash.as_bytes().as_slice(),
        wire_sha256
    );
    assert_ne!(
        context.analysis_content_hash.as_bytes().as_slice(),
        wire_sha256
    );

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "the test keeps the complete compaction wire assertions in one vertical slice"
)]
#[tokio::test]
async fn compaction_is_wire_exact_and_transport_only() -> TestResult {
    let arrivals = Arc::new(Mutex::new(Vec::<Arrival>::new()));
    let upstream_arrivals = Arc::clone(&arrivals);
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_address = upstream_listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/responses/compact",
            post(move |request: Request| {
                let upstream_arrivals = Arc::clone(&upstream_arrivals);
                async move {
                    let (parts, body) = request.into_parts();
                    let body = to_bytes(body, 8 * 1024 * 1024).await.unwrap_or_default();
                    if let Ok(mut arrivals) = upstream_arrivals.lock() {
                        arrivals.push(Arrival {
                            path: parts.uri.path().to_owned(),
                            body,
                            authorization: parts
                                .headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            content_encoding: parts
                                .headers
                                .get("content-encoding")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            content_length: parts
                                .headers
                                .get("content-length")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                        });
                    }
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/octet-stream")
                        .body(Body::from("compact-result"))
                        .unwrap_or_else(|_error| Response::new(Body::empty()))
                }
            }),
        );
        let _result = axum::serve(upstream_listener, app).await;
    });

    let sink = Arc::new(Sink::default());
    let proxy = TransparentProxy::new(ProxyConfig::new(
        ProviderEndpoint::new(&format!("http://{upstream_address}/v1/responses/compact"))?,
        limits()?,
        ContextAnalysisMode::Shadow,
    )?)?
    .with_observation_sink(Arc::<Sink>::clone(&sink));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _result = axum::serve(proxy_listener, proxy.router()).await;
    });

    let json = br#"{"previous_response_id":"resp_old","input":[{"type":"input_text","text":"compact me"}]}"#;
    let wire = zstd::stream::encode_all(std::io::Cursor::new(json), 1)?;
    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/responses/compact"))
        .header("content-encoding", "zstd")
        .header(
            "authorization",
            "Bearer TRACEPRESS_SUBSCRIPTION_SECRET_CANARY",
        )
        .body(wire.clone())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?.as_ref(), b"compact-result");
    wait_for_compaction(&sink).await?;

    let arrival = arrivals
        .lock()
        .map_err(|_error| "upstream lock poisoned")?
        .first()
        .cloned()
        .ok_or("upstream did not receive compact request")?;
    assert_eq!(arrival.path, "/v1/responses/compact");
    assert_eq!(arrival.body.as_ref(), wire.as_slice());
    assert_eq!(arrival.content_encoding.as_deref(), Some("zstd"));
    assert_eq!(
        arrival.authorization.as_deref(),
        Some("Bearer TRACEPRESS_SUBSCRIPTION_SECRET_CANARY")
    );

    assert!(
        sink.requests
            .lock()
            .map_err(|_| "request lock poisoned")?
            .is_empty()
    );
    assert!(
        sink.contexts
            .lock()
            .map_err(|_| "context lock poisoned")?
            .is_empty()
    );
    let observation = sink
        .compactions
        .lock()
        .map_err(|_error| "compaction lock poisoned")?
        .first()
        .cloned()
        .ok_or("compaction observation missing")?;
    assert_eq!(observation.request_bytes, wire.len() as u64);
    assert_eq!(observation.response_bytes, "compact-result".len() as u64);
    assert_eq!(observation.status_code, Some(200));
    assert_eq!(
        observation.outcome,
        tracepress_proxy::CompactionOutcome::Completed
    );
    assert_eq!(observation.content_encoding, ContentEncoding::Zstd);
    assert!(observation.duration_us.is_some());

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn compaction_provider_error_is_forwarded_and_marked_failed() -> TestResult {
    let arrivals = Arc::new(Mutex::new(Vec::<Arrival>::new()));
    let upstream_arrivals = Arc::clone(&arrivals);
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_address = upstream_listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/responses/compact",
            post(move |request: Request| {
                let upstream_arrivals = Arc::clone(&upstream_arrivals);
                async move {
                    let (parts, body) = request.into_parts();
                    let body = to_bytes(body, 8 * 1024 * 1024).await.unwrap_or_default();
                    if let Ok(mut arrivals) = upstream_arrivals.lock() {
                        arrivals.push(Arrival {
                            path: parts.uri.path().to_owned(),
                            body,
                            authorization: parts
                                .headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            content_encoding: parts
                                .headers
                                .get("content-encoding")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            content_length: parts
                                .headers
                                .get("content-length")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                        });
                    }
                    Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"error":"retry"}"#))
                        .unwrap_or_else(|_error| Response::new(Body::empty()))
                }
            }),
        );
        let _result = axum::serve(upstream_listener, app).await;
    });

    let sink = Arc::new(Sink::default());
    let proxy = TransparentProxy::new(ProxyConfig::new(
        ProviderEndpoint::new(&format!("http://{upstream_address}/v1/responses/compact"))?,
        limits()?,
        ContextAnalysisMode::Shadow,
    )?)?
    .with_observation_sink(Arc::<Sink>::clone(&sink));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _result = axum::serve(proxy_listener, proxy.router()).await;
    });

    let request_body = Bytes::from_static(br#"{"input":[]}"#);
    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/responses/compact"))
        .header(
            "authorization",
            "Bearer TRACEPRESS_SUBSCRIPTION_SECRET_CANARY",
        )
        .body(request_body.clone())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.bytes().await?.as_ref(), br#"{"error":"retry"}"#);
    wait_for_compaction(&sink).await?;

    let arrival = arrivals
        .lock()
        .map_err(|_error| "upstream lock poisoned")?
        .first()
        .cloned()
        .ok_or("upstream did not receive compact request")?;
    assert_eq!(arrival.path, "/v1/responses/compact");
    assert_eq!(arrival.body, request_body);
    assert_eq!(
        arrival.authorization.as_deref(),
        Some("Bearer TRACEPRESS_SUBSCRIPTION_SECRET_CANARY")
    );

    let observation = sink
        .compactions
        .lock()
        .map_err(|_error| "compaction lock poisoned")?
        .first()
        .cloned()
        .ok_or("compaction observation missing")?;
    assert_eq!(observation.status_code, Some(429));
    assert_eq!(
        observation.response_bytes,
        br#"{"error":"retry"}"#.len() as u64
    );
    assert_eq!(
        observation.outcome,
        tracepress_proxy::CompactionOutcome::Failed
    );

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn upstream_redirect_is_returned_without_following_or_leaking_auth() -> TestResult {
    let attacker_hits = Arc::new(AtomicUsize::new(0));
    let attacker_hits_for_server = Arc::clone(&attacker_hits);
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_address = upstream_listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        let redirect_target = format!("http://{upstream_address}/attacker");
        let app = Router::new()
            .route(
                "/v1/responses",
                post(move |_request: Request| {
                    let redirect_target = redirect_target.clone();
                    async move {
                        Response::builder()
                            .status(StatusCode::FOUND)
                            .header("location", redirect_target)
                            .body(Body::empty())
                            .unwrap_or_else(|_error| Response::new(Body::empty()))
                    }
                }),
            )
            .route(
                "/attacker",
                post(move |request: Request| {
                    let attacker_hits_for_server = Arc::clone(&attacker_hits_for_server);
                    async move {
                        let _ = request;
                        let _ = attacker_hits_for_server.fetch_add(1, Ordering::Relaxed);
                        StatusCode::OK
                    }
                }),
            );
        let _result = axum::serve(upstream_listener, app).await;
    });

    let proxy = TransparentProxy::new(ProxyConfig::new(
        ProviderEndpoint::new(&format!("http://{upstream_address}/v1/responses"))?,
        limits()?,
        ContextAnalysisMode::Off,
    )?)?;
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _result = axum::serve(proxy_listener, proxy.router()).await;
    });
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .post(format!("http://{proxy_address}/v1/responses"))
        .header(
            "authorization",
            "Bearer TRACEPRESS_SUBSCRIPTION_SECRET_CANARY",
        )
        .body(r#"{"model":"gpt-5"}"#)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::FOUND);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert_eq!(attacker_hits.load(Ordering::Relaxed), 0);

    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}
