//! Real HTTP tests for transparent Phase 1 forwarding.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use tokio::net::{TcpListener, TcpStream};
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes};
use tracepress_provider::ProviderEndpoint;
use tracepress_proxy::{
    ForwardMetadata, MetadataSink, MetadataSinkError, ProxyConfig, TransparentProxy,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone)]
struct UpstreamState {
    response_status: StatusCode,
    response_body: Bytes,
    observed: Arc<Mutex<Option<ObservedRequest>>>,
}

#[derive(Clone, Debug)]
struct ObservedRequest {
    body: Bytes,
    authorization: Option<String>,
}

#[derive(Debug, Default)]
struct RecordingSink(Mutex<Vec<ForwardMetadata>>);

impl MetadataSink for RecordingSink {
    fn try_record(&self, metadata: ForwardMetadata) -> Result<(), MetadataSinkError> {
        if let Ok(mut records) = self.0.lock() {
            records.push(metadata);
        }
        Ok(())
    }
}

#[tokio::test]
async fn transparent_chat_completion_bytes() -> TestResult {
    // Given: real upstream and proxy sockets plus opaque non-UTF-8 request/response bodies.
    let request_body = Bytes::from_static(b"{\"input\":\"opaque\"}\0\xff");
    let response_body = Bytes::from_static(b"data: {\"delta\":1}\n\n\0\xfe");
    let (upstream, observed, upstream_task) =
        spawn_upstream(StatusCode::OK, response_body.clone()).await?;
    let sink = Arc::new(RecordingSink::default());
    let (proxy, proxy_task) =
        spawn_proxy(upstream, 4_096, 4_096, Arc::<RecordingSink>::clone(&sink)).await?;

    // When: the client sends credentials and opaque bytes through the proxy.
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/chat/completions"))
        .header("authorization", "Bearer top-secret")
        .header("content-type", "application/octet-stream")
        .body(request_body.clone())
        .send()
        .await?;
    let status = response.status();
    let received = response.bytes().await?;

    // Then: bodies and status are exact, credentials reached only the upstream, and metadata is safe.
    assert_eq!(status, StatusCode::OK);
    assert_eq!(received, response_body);
    let observed_guard = observed
        .lock()
        .map_err(|_| "upstream observation poisoned")?;
    let observed_request = observed_guard
        .as_ref()
        .cloned()
        .ok_or("upstream did not observe request")?;
    drop(observed_guard);
    assert_eq!(observed_request.body, request_body);
    assert_eq!(
        observed_request.authorization.as_deref(),
        Some("Bearer top-secret")
    );
    let records = sink.0.lock().map_err(|_| "metadata sink poisoned")?;
    let record = records
        .first()
        .copied()
        .ok_or("metadata was not recorded")?;
    let metadata_debug = format!("{records:?}");
    drop(records);
    assert_eq!(record.request_bytes, u64::try_from(request_body.len())?);
    assert_eq!(record.status_code, Some(200));
    assert_eq!(record.response_bytes, None);
    assert!(!metadata_debug.contains("top-secret"));
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn upstream_429_500_cancel_and_malformed() -> TestResult {
    // Given/When/Then: provider error statuses pass through with their exact bodies.
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let body = Bytes::from(format!("status-{}\0", status.as_u16()));
        let (upstream, _observed, upstream_task) = spawn_upstream(status, body.clone()).await?;
        let (proxy, proxy_task) =
            spawn_proxy(upstream, 1_024, 1_024, Arc::new(RecordingSink::default())).await?;
        let response = reqwest::Client::new()
            .post(format!("{proxy}/v1/chat/completions"))
            .body(Bytes::from_static(b"request"))
            .send()
            .await?;
        assert_eq!(response.status(), status);
        assert_eq!(response.bytes().await?, body);
        proxy_task.abort();
        upstream_task.abort();
    }

    // Given/When/Then: malformed upstream transport becomes an explicit 502 without body mutation.
    let malformed_listener = TcpListener::bind("127.0.0.1:0").await?;
    let malformed_address = malformed_listener.local_addr()?;
    let malformed_task = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        if let Ok((mut stream, _peer)) = malformed_listener.accept().await {
            let _result = stream.write_all(b"not-http\r\n\r\n").await;
        }
    });
    let endpoint =
        ProviderEndpoint::new(&format!("http://{malformed_address}/v1/chat/completions"))?;
    let (proxy, proxy_task) =
        spawn_proxy(endpoint, 1_024, 1_024, Arc::new(RecordingSink::default())).await?;
    let response = reqwest::Client::new()
        .post(format!("{proxy}/v1/chat/completions"))
        .body("request")
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    proxy_task.abort();
    malformed_task.abort();

    // Given/When/Then: dropping the downstream request cancels an unfinished upstream exchange.
    let cancelled = Arc::new(Mutex::new(false));
    let cancellation_guard = Arc::<Mutex<bool>>::clone(&cancelled);
    let slow_listener = TcpListener::bind("127.0.0.1:0").await?;
    let slow_address = slow_listener.local_addr()?;
    let slow_router = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let guard = CancellationGuard(Arc::<Mutex<bool>>::clone(&cancellation_guard));
            async move {
                let _guard = guard;
                std::future::pending::<Response>().await
            }
        }),
    );
    let slow_task = tokio::spawn(async move {
        let _result = axum::serve(slow_listener, slow_router).await;
    });
    let endpoint = ProviderEndpoint::new(&format!("http://{slow_address}/v1/chat/completions"))?;
    let (proxy, proxy_task) =
        spawn_proxy(endpoint, 1_024, 1_024, Arc::new(RecordingSink::default())).await?;
    let request_task = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{proxy}/v1/chat/completions"))
            .body("request")
            .send()
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    request_task.abort();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if cancelled.lock().is_ok_and(|value| *value) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    proxy_task.abort();
    slow_task.abort();
    Ok(())
}

struct CancellationGuard(Arc<Mutex<bool>>);

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if let Ok(mut cancelled) = self.0.lock() {
            *cancelled = true;
        }
    }
}

async fn upstream_handler(State(state): State<UpstreamState>, request: Request) -> Response {
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(request.into_body(), 16 * 1_024)
        .await
        .unwrap_or_default();
    if let Ok(mut observed) = state.observed.lock() {
        *observed = Some(ObservedRequest {
            body,
            authorization,
        });
    }
    Response::builder()
        .status(state.response_status)
        .header("content-type", "application/octet-stream")
        .body(Body::from(state.response_body))
        .unwrap_or_else(|_error| Response::new(Body::empty()))
}

async fn spawn_upstream(
    status: StatusCode,
    body: Bytes,
) -> Result<
    (
        ProviderEndpoint,
        Arc<Mutex<Option<ObservedRequest>>>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let observed = Arc::new(Mutex::new(None));
    let state = UpstreamState {
        response_status: status,
        response_body: body,
        observed: Arc::<Mutex<Option<ObservedRequest>>>::clone(&observed),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new()
        .route("/v1/chat/completions", post(upstream_handler))
        .with_state(state);
    let task = tokio::spawn(async move {
        let _result = axum::serve(listener, router).await;
    });
    let endpoint = ProviderEndpoint::new(&format!("http://{address}/v1/chat/completions"))?;
    Ok((endpoint, observed, task))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the test server keeps endpoint, independent limits, and metadata sink explicit"
)]
async fn spawn_proxy(
    upstream: ProviderEndpoint,
    request_limit: u64,
    response_limit: u64,
    sink: Arc<dyn MetadataSink>,
) -> Result<(String, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
    let proxy = TransparentProxy::new(ProxyConfig::new(
        upstream,
        MaxRequestBodyBytes::new(request_limit)?,
        MaxResponseBodyBytes::new(response_limit)?,
    ))?
    .with_metadata_sink(sink);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _result = axum::serve(listener, proxy.router()).await;
    });
    wait_until_listening(address).await?;
    Ok((format!("http://{address}"), task))
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
