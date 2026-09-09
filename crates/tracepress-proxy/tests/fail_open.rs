//! Auxiliary metadata failures must not change transparent forwarding.

use std::sync::Arc;

use axum::{Router, body::Bytes, extract::Request, http::StatusCode, routing::post};
use tokio::net::TcpListener;
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes};
use tracepress_provider::ProviderEndpoint;
use tracepress_proxy::{
    ForwardMetadata, MetadataSink, MetadataSinkError, ProxyConfig, TransparentProxy,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Default)]
struct FailingSink;

impl MetadataSink for FailingSink {
    fn try_record(&self, _metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        Err(MetadataSinkError::rejected())
    }
}

#[tokio::test]
async fn metadata_sink_failure_does_not_block_or_mutate_response() -> TestResult {
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_address = upstream_listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|_request: Request| async {
                (StatusCode::OK, Bytes::from_static(b"opaque\0response"))
            }),
        );
        axum::serve(upstream_listener, app).await
    });
    let endpoint =
        ProviderEndpoint::new(&format!("http://{upstream_address}/v1/chat/completions"))?;
    let sink = FailingSink;
    let proxy = TransparentProxy::new(ProxyConfig::new(
        endpoint,
        MaxRequestBodyBytes::new(1_024)?,
        MaxResponseBodyBytes::new(1_024)?,
    ))?
    .with_metadata_sink(Arc::new(sink));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move { axum::serve(proxy_listener, proxy.router()).await });

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/chat/completions"))
        .body(Bytes::from_static(b"request\0bytes"))
        .send()
        .await?;
    let status = response.status();
    let body = response.bytes().await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"opaque\0response");
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}
