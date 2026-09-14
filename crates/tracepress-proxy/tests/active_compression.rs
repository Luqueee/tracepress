//! Explicit Phase 4.2 active-arm tests.
//!
//! The default proxy remains covered by the transparent byte-exact suites. These tests opt into
//! the active mode explicitly and verify that only the selected `ToolResult` JSON value changes.

#![allow(
    clippy::indexing_slicing,
    reason = "JSON assertions use fixed test fixture paths"
)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use tracepress_core::{ResourceLimits, ResourceLimitsConfig};
use tracepress_provider::{ProviderEndpoint, ResponseObservation};
use tracepress_proxy::{
    ActiveCompressionMode, ActiveCompressionObservation, ContextAnalysisMode, ForwardId,
    ForwardMetadata, MetadataSink, MetadataSinkError, ObservationSinkError,
    ProviderObservationSink, ProxyConfig, RequestContextObservation, TransparentProxy,
    TransportFailure,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn resource_limits() -> Result<ResourceLimits, Box<dyn std::error::Error>> {
    Ok(ResourceLimits::try_from(ResourceLimitsConfig {
        max_raw_bytes: Some(65_536),
        max_request_body_bytes: Some(65_536),
        max_response_body_bytes: Some(65_536),
        max_decompressed_bytes: Some(65_536),
        max_ipc_frame_bytes: Some(65_536),
        max_ipc_queue_items: Some(128),
        max_json_nesting: Some(64),
        max_json_items: Some(100_000),
        max_line_bytes: Some(65_536),
        max_processing_time_ms: Some(250),
        max_cpu_work_units: Some(1_000_000),
    })?)
}

#[derive(Clone, Default)]
struct StateCapture {
    body: Arc<Mutex<Option<Bytes>>>,
    content_encoding: Arc<Mutex<Option<String>>>,
}

async fn upstream(State(state): State<StateCapture>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 65_536)
        .await
        .unwrap_or_else(|_| Bytes::new());
    if let Ok(mut captured) = state.body.lock() {
        *captured = Some(body);
    }
    if let Ok(mut captured) = state.content_encoding.lock() {
        *captured = parts
            .headers
            .get("content-encoding")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(Bytes::from_static(
            br#"{"id":"active-test","status":"completed","output":[]}"#,
        )))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

#[derive(Default)]
struct ObservationCapture {
    active: Mutex<Vec<ActiveCompressionObservation>>,
}

impl MetadataSink for ObservationCapture {
    fn try_record(&self, _metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        Ok(())
    }
}

impl ProviderObservationSink for ObservationCapture {
    fn try_record_request_context(
        &self,
        _observation: RequestContextObservation,
    ) -> Result<(), ObservationSinkError> {
        Ok(())
    }

    fn try_record_context_analysis(
        &self,
        _observation: tracepress_proxy::ContextAnalysisObservation,
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

    fn try_record_active_compression(
        &self,
        observation: ActiveCompressionObservation,
    ) -> Result<(), ObservationSinkError> {
        self.active
            .lock()
            .map_err(|_| ObservationSinkError::rejected())?
            .push(observation);
        Ok(())
    }
}

#[tokio::test]
async fn explicit_active_mode_rewrites_only_tool_result_json() -> TestResult {
    let capture = StateCapture::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let upstream_task = tokio::spawn({
        let state = capture.clone();
        async move {
            let _ = axum::serve(
                listener,
                Router::new()
                    .route("/v1/responses", post(upstream))
                    .with_state(state),
            )
            .await;
        }
    });

    let sink = Arc::new(ObservationCapture::default());
    let proxy = TransparentProxy::new(
        ProxyConfig::new(
            ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?,
            resource_limits()?,
            ContextAnalysisMode::Shadow,
        )?
        .with_active_compression_mode(ActiveCompressionMode::JsonMinify),
    )?
    .with_metadata_sink(Arc::clone(&sink) as Arc<dyn MetadataSink>)
    .with_observation_sink(Arc::clone(&sink) as Arc<dyn ProviderObservationSink>);
    let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _ = axum::serve(proxy_listener, proxy.router()).await;
    });

    let original = br#"{"model":"test","stream":false,"input":[{"type":"function_call_output","call_id":"x","output":"[{ \"name\": \"a\", \"status\": \"ok\", \"size\": 10 }, { \"name\": \"b\", \"status\": \"ok\", \"size\": 12 }]"}],"keep":"exact"}"#;
    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/responses"))
        .header("content-type", "application/json")
        .body(original.as_slice())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await?;

    let forwarded = capture
        .body
        .lock()
        .map_err(|_| "capture poisoned")?
        .clone()
        .ok_or("upstream did not receive a request")?;
    let forwarded_value: serde_json::Value = serde_json::from_slice(&forwarded)?;
    assert_eq!(
        forwarded_value["keep"],
        serde_json::Value::String("exact".to_owned())
    );
    assert_eq!(
        forwarded_value["input"][0]["output"],
        serde_json::Value::String(
            r#"[{"name":"a","status":"ok","size":10},{"name":"b","status":"ok","size":12}]"#
                .to_owned(),
        )
    );
    assert_ne!(forwarded.as_ref(), original);

    let active = sink.active.lock().map_err(|_| "active capture poisoned")?;
    assert_eq!(active.len(), 1);
    let metrics = &active[0].metrics;
    assert_eq!(
        metrics.status,
        tracepress_compression::ActiveRewriteStatus::Rewritten
    );
    assert_eq!(metrics.rewrites, 1);
    assert!(metrics.recovery_verified);
    assert!(metrics.deterministic);
    assert!(metrics.output_bytes.unwrap_or_default() < metrics.input_bytes);
    drop(active);
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn explicit_search_mode_rewrites_only_search_tool_result_text() -> TestResult {
    let capture = StateCapture::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let upstream_task = tokio::spawn({
        let state = capture.clone();
        async move {
            let _ = axum::serve(
                listener,
                Router::new()
                    .route("/v1/responses", post(upstream))
                    .with_state(state),
            )
            .await;
        }
    });

    let sink = Arc::new(ObservationCapture::default());
    let proxy = TransparentProxy::new(
        ProxyConfig::new(
            ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?,
            resource_limits()?,
            ContextAnalysisMode::Shadow,
        )?
        .with_active_compression_mode(ActiveCompressionMode::SearchProjection),
    )?
    .with_metadata_sink(Arc::clone(&sink) as Arc<dyn MetadataSink>)
    .with_observation_sink(Arc::clone(&sink) as Arc<dyn ProviderObservationSink>);
    let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _ = axum::serve(proxy_listener, proxy.router()).await;
    });

    let original = br#"{"model":"test","input":[{"type":"function_call_output","call_id":"x","output":"src/a.rs:10:foo\nsrc/a.rs:11:bar\nsrc/a.rs:12:baz\nsrc/b.rs:2:foo\nsrc/b.rs:3:bar\nsrc/b.rs:4:baz\n"}]}"#;
    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/responses"))
        .header("content-type", "application/json")
        .body(original.as_slice())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await?;

    let forwarded = capture
        .body
        .lock()
        .map_err(|_| "capture poisoned")?
        .clone()
        .ok_or("upstream did not receive a request")?;
    let forwarded_value: serde_json::Value = serde_json::from_slice(&forwarded)?;
    let projected = forwarded_value["input"][0]["output"]
        .as_str()
        .ok_or("projected search result was not a string")?;
    assert!(projected.starts_with("[search results]\nsrc/a.rs:\n"));
    assert_ne!(forwarded.as_ref(), original);

    let active = sink.active.lock().map_err(|_| "active capture poisoned")?;
    assert_eq!(active.len(), 1);
    let metrics = &active[0].metrics;
    assert_eq!(
        metrics.compressor_id,
        "search.result_projection".to_owned()
    );
    assert_eq!(metrics.status, tracepress_compression::ActiveRewriteStatus::Rewritten);
    assert!(metrics.recovery_verified);
    assert!(metrics.deterministic);
    drop(active);
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn active_mode_is_not_enabled_by_default() -> TestResult {
    let limits = resource_limits()?;
    let endpoint = ProviderEndpoint::new("http://127.0.0.1:1/v1/responses")?;
    let config = ProxyConfig::new(endpoint, limits, ContextAnalysisMode::Shadow)?;
    assert_eq!(config.active_compression_mode, ActiveCompressionMode::Off);
    Ok(())
}

#[tokio::test]
async fn explicit_active_mode_rewrites_zstd_wire_and_preserves_encoding() -> TestResult {
    let capture = StateCapture::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let upstream_task = tokio::spawn({
        let state = capture.clone();
        async move {
            let _ = axum::serve(
                listener,
                Router::new()
                    .route("/v1/responses", post(upstream))
                    .with_state(state),
            )
            .await;
        }
    });

    let sink = Arc::new(ObservationCapture::default());
    let proxy = TransparentProxy::new(
        ProxyConfig::new(
            ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?,
            resource_limits()?,
            ContextAnalysisMode::Shadow,
        )?
        .with_active_compression_mode(ActiveCompressionMode::JsonMinify),
    )?
    .with_metadata_sink(Arc::clone(&sink) as Arc<dyn MetadataSink>)
    .with_observation_sink(Arc::clone(&sink) as Arc<dyn ProviderObservationSink>);
    let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _ = axum::serve(proxy_listener, proxy.router()).await;
    });

    let original = br#"{"model":"test","stream":false,"input":[{"type":"function_call_output","call_id":"x","output":"[{ \"name\": \"alpha\", \"status\": \"ok\", \"size\": 10 }, { \"name\": \"bravo\", \"status\": \"ok\", \"size\": 12 }, { \"name\": \"charlie\", \"status\": \"ok\", \"size\": 14 }]"}],"keep":"exact"}"#;
    let wire = zstd::stream::encode_all(std::io::Cursor::new(original), 1)?;
    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/responses"))
        .header("content-type", "application/json")
        .header("content-encoding", "zstd")
        .body(wire.clone())
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await?;

    let forwarded = capture
        .body
        .lock()
        .map_err(|_| "capture poisoned")?
        .clone()
        .ok_or("upstream did not receive a request")?;
    let forwarded_decoded = zstd::stream::decode_all(std::io::Cursor::new(forwarded.as_ref()))?;
    let forwarded_value: serde_json::Value = serde_json::from_slice(&forwarded_decoded)?;
    assert_eq!(
        forwarded_value["keep"],
        serde_json::Value::String("exact".to_owned())
    );
    assert_eq!(
        forwarded_value["input"][0]["output"],
        serde_json::Value::String(
            r#"[{"name":"alpha","status":"ok","size":10},{"name":"bravo","status":"ok","size":12},{"name":"charlie","status":"ok","size":14}]"#
                .to_owned(),
        )
    );
    assert_eq!(
        capture
            .content_encoding
            .lock()
            .map_err(|_| "encoding capture poisoned")?
            .as_deref(),
        Some("zstd")
    );
    assert_ne!(forwarded.as_ref(), wire.as_slice());

    let active = sink.active.lock().map_err(|_| "active capture poisoned")?;
    assert_eq!(active.len(), 1);
    let metrics = &active[0].metrics;
    assert_eq!(
        metrics.status,
        tracepress_compression::ActiveRewriteStatus::Rewritten
    );
    assert_eq!(metrics.rewrites, 1);
    assert!(metrics.recovery_verified);
    assert!(metrics.deterministic);
    assert!(metrics.output_bytes.unwrap_or_default() < metrics.input_bytes);
    drop(active);
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}
