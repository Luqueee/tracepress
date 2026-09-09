#![allow(
    clippy::multiple_crate_versions,
    reason = "the HTTP client and TLS stack currently require distinct transitive platform crates"
)]

//! Transparent OpenAI-compatible HTTP forwarding for Tracepress Phase 1.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::Response;
use axum::routing::post;
use futures_util::StreamExt as _;
use thiserror::Error;
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes};
use tracepress_provider::ProviderEndpoint;

/// Allowlisted transport facts emitted without headers or body content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ForwardMetadata {
    /// Exact accepted request body length.
    pub request_bytes: u64,
    /// Upstream status when transport reached response headers.
    pub status_code: Option<u16>,
    /// Response length remains unknown until the stream completes.
    pub response_bytes: Option<u64>,
}

impl ForwardMetadata {
    const fn new(request_bytes: u64, status_code: Option<u16>) -> Self {
        Self {
            request_bytes,
            status_code,
            response_bytes: None,
        }
    }
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

/// Synchronous, non-blocking boundary for optional transport metadata.
pub trait MetadataSink: Send + Sync + 'static {
    /// Attempts to accept allowlisted metadata without affecting forwarding.
    ///
    /// # Errors
    /// Returns a sink-local failure that the proxy deliberately ignores.
    fn try_record(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError>;
}

#[derive(Debug)]
struct NoopMetadataSink;

impl MetadataSink for NoopMetadataSink {
    fn try_record(&self, _metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        Ok(())
    }
}

/// Validated transparent proxy configuration.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// Full upstream chat-completions endpoint.
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
        })
    }

    /// Installs an allowlisted metadata sink. Sink behavior never gates forwarding.
    #[must_use]
    pub fn with_metadata_sink(mut self, metadata: Arc<dyn MetadataSink>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Builds the sole Phase 1 HTTP route.
    pub fn router(self) -> Router {
        Router::new()
            .route("/v1/chat/completions", post(forward))
            .with_state(self)
    }
}

async fn forward(State(proxy): State<TransparentProxy>, request: Request) -> Response {
    match forward_inner(&proxy, request).await {
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
) -> Result<Response, ForwardError> {
    let (parts, body) = request.into_parts();
    let maximum = usize::try_from(proxy.config.max_request_body_bytes.get())
        .map_err(|_error| ForwardError::RequestLimitUnrepresentable)?;
    let bytes = to_bytes(body, maximum)
        .await
        .map_err(|_error| ForwardError::RequestTooLarge)?;
    let request_bytes =
        u64::try_from(bytes.len()).map_err(|_error| ForwardError::RequestLimitUnrepresentable)?;
    let headers = forwarded_headers(&parts.headers);
    let upstream = proxy
        .client
        .post(proxy.config.upstream.as_str())
        .headers(headers)
        .body(bytes)
        .send()
        .await
        .map_err(ForwardError::Upstream)?;
    let status = upstream.status();
    let _metadata_result = proxy
        .metadata
        .try_record(ForwardMetadata::new(request_bytes, Some(status.as_u16())));
    let response_headers = forwarded_headers(upstream.headers());
    let maximum_response = proxy.config.max_response_body_bytes.get();
    let stream = upstream
        .bytes_stream()
        .scan((0_u64, false), move |(seen, finished), item| {
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
        });
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok(response)
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
    /// Upstream transport failed before a response was available.
    #[error("upstream transport failed")]
    Upstream(#[source] reqwest::Error),
}

impl ForwardError {
    const fn public_message(&self) -> &'static str {
        match self {
            Self::RequestTooLarge => "request body exceeds configured limit",
            Self::ClientBuild(_) | Self::RequestLimitUnrepresentable | Self::Upstream(_) => {
                "upstream transport failed"
            }
        }
    }
}
