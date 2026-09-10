//! A buffered document response is interpreted under a byte bound that reaches 32 MiB, so that
//! parse must never run on the runtime worker the forwarding tasks are polled by.
//!
//! This lives in its own test binary because it measures scheduling latency: a sibling test that
//! deliberately saturates every core would otherwise be part of the measurement.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::post;
use tokio::net::{TcpListener, TcpStream};
use tracepress_core::{ResourceLimits, ResourceLimitsConfig};
use tracepress_provider::{ProviderEndpoint, ResponseObservation};
use tracepress_proxy::{
    ForwardId, ObservationSinkError, ProviderObservationSink, ProxyConfig,
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

/// Output items per buffered document, sized to the parser's bounded item budget.
const DOCUMENT_OUTPUT_ITEMS: usize = 24_750;
/// Concurrent buffered documents whose parses would otherwise hold the runtime worker.
const DOCUMENT_FORWARDS: usize = 8;
/// Interval of the heartbeat that samples the proxy runtime's own worker.
const HEARTBEAT_TICK: Duration = Duration::from_millis(2);
/// Heartbeat samples, covering the window the documents are observed in.
const HEARTBEAT_TICKS: usize = 60;
/// Upper bound on how late one heartbeat tick may be woken.
///
/// One of these documents costs about 35 ms to interpret, so a parse left on the async observer
/// task cannot yield to a 2 ms timer inside this budget, while a detached parse never holds the
/// worker at all. Measured on this workload: ~1 ms detached, ~35 ms on the async task.
const HEARTBEAT_BUDGET: Duration = Duration::from_millis(15);

/// Request body of every forward this workload drives.
const DOCUMENT_REQUEST: &str = r#"{"model":"document","stream":false}"#;

/// A completed non-streamed response whose bounded parse costs real CPU.
fn large_document_response() -> Bytes {
    let items = std::iter::repeat_n(
        r#"{"type":"output_text","text":"x"}"#,
        DOCUMENT_OUTPUT_ITEMS,
    )
    .collect::<Vec<_>>()
    .join(",");
    Bytes::from(format!(
        r#"{{"id":"resp_document","model":"m","status":"completed","output":[{items}],"usage":{{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}}"#
    ))
}

#[tokio::test]
async fn semantic_response_parsing_never_occupies_the_runtime_worker() -> TestResult {
    let response_body = large_document_response();
    let upstream = Upstream::spawn(response_body.clone()).await?;
    let sink = Arc::new(CountingSink::default());
    // The proxy owns a single-worker runtime of its own, so a heartbeat on that runtime samples
    // exactly the worker every forwarding task is polled by, with neither the clients nor the
    // upstream inside the measurement.
    let isolated = spawn_isolated_proxy(Arc::<CountingSink>::clone(&sink), &upstream.endpoint)?;
    let heartbeat = isolated.worker.spawn(async move {
        let mut worst = Duration::ZERO;
        for _tick in 0..HEARTBEAT_TICKS {
            let woken = Instant::now();
            tokio::time::sleep(HEARTBEAT_TICK).await;
            worst = worst.max(woken.elapsed().saturating_sub(HEARTBEAT_TICK));
        }
        worst
    });

    let client = reqwest::Client::new();
    let forwards = (0..DOCUMENT_FORWARDS)
        .map(|_forward| spawn_forward(&client, &isolated.base_url))
        .collect::<Vec<_>>();
    for forward in forwards {
        let (status, bytes) = forward.await??;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes.as_ref(), response_body.as_ref());
    }
    upstream.wait_for_exact_requests(DOCUMENT_FORWARDS).await?;
    sink.wait_for_responses(DOCUMENT_FORWARDS).await?;

    let worst_tick = heartbeat.await?;
    let worst_tick_ms = worst_tick.as_millis();
    assert!(
        worst_tick < HEARTBEAT_BUDGET,
        "the proxy's worker was held for {worst_tick_ms}ms, so response parsing is still on the async task"
    );

    isolated.stop()?;
    upstream.stop();
    Ok(())
}

/// Counts observations without holding anything the forwarding path waits on.
#[derive(Default)]
struct CountingSink {
    requests: AtomicUsize,
    responses: AtomicUsize,
}

impl CountingSink {
    async fn wait_for_responses(&self, expected: usize) -> TestResult {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.responses.load(Ordering::Relaxed) < expected {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        assert_eq!(self.requests.load(Ordering::Relaxed), expected);
        Ok(())
    }
}

impl ProviderObservationSink for CountingSink {
    fn try_record_request_context(
        &self,
        _context: RequestContextObservation,
    ) -> Result<(), ObservationSinkError> {
        let _previous = self.requests.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn try_record_response(
        &self,
        _forward: ForwardId,
        _observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        let _previous = self.responses.fetch_add(1, Ordering::Relaxed);
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

/// Answers every forward with the same buffered document, counting byte-exact arrivals.
#[derive(Clone)]
struct UpstreamState {
    body: Bytes,
    exact_requests: Arc<AtomicUsize>,
}

struct Upstream {
    endpoint: ProviderEndpoint,
    exact_requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Upstream {
    async fn spawn(body: Bytes) -> Result<Self, Box<dyn std::error::Error>> {
        let exact_requests = Arc::new(AtomicUsize::new(0));
        let state = UpstreamState {
            body,
            exact_requests: Arc::clone(&exact_requests),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let router = Router::new()
            .route("/v1/responses", post(answer_document))
            .with_state(state);
        let task = tokio::spawn(async move {
            let _served = axum::serve(listener, router).await;
        });
        wait_until_listening(address).await?;
        Ok(Self {
            endpoint: ProviderEndpoint::new(&format!("http://{address}/v1/responses"))?,
            exact_requests,
            task,
        })
    }

    async fn wait_for_exact_requests(&self, expected: usize) -> TestResult {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.exact_requests.load(Ordering::Relaxed) < expected {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        Ok(())
    }

    fn stop(self) {
        self.task.abort();
    }
}

async fn answer_document(State(state): State<UpstreamState>, request: Request) -> Response {
    let (_parts, body) = request.into_parts();
    let received = to_bytes(body, 1024 * 1024).await.unwrap_or_default();
    if received.as_ref() == DOCUMENT_REQUEST.as_bytes() {
        let _previous = state.exact_requests.fetch_add(1, Ordering::Relaxed);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, state.body.len())
        .body(Body::from(state.body))
        .unwrap_or_else(|_error| Response::new(Body::empty()))
}

/// A proxy served by a runtime of its own, so occupying its worker is observable.
struct IsolatedProxy {
    base_url: String,
    worker: tokio::runtime::Handle,
    shutdown: tokio::sync::oneshot::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl IsolatedProxy {
    fn stop(self) -> TestResult {
        let _sent = self.shutdown.send(());
        self.thread
            .join()
            .map_err(|_error| "isolated proxy thread panicked")?;
        Ok(())
    }
}

/// Serves the proxy from a dedicated single-worker runtime on its own thread.
fn spawn_isolated_proxy(
    sink: Arc<CountingSink>,
    upstream: &ProviderEndpoint,
) -> Result<IsolatedProxy, Box<dyn std::error::Error>> {
    let proxy = TransparentProxy::new(ProxyConfig::new(
        upstream.clone(),
        resource_limits(4 * 1024 * 1024, 4 * 1024 * 1024)?,
    )?)?
    .with_observation_sink(sink);
    // Binding before the thread starts publishes the address without a readiness handshake.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let (shutdown, halt) = tokio::sync::oneshot::channel::<()>();
    let (ready, started) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        if ready.send(runtime.handle().clone()).is_err() {
            return;
        }
        runtime.block_on(async move {
            let Ok(listener) = TcpListener::from_std(listener) else {
                return;
            };
            let _served = axum::serve(listener, proxy.router())
                .with_graceful_shutdown(async move {
                    drop(halt.await);
                })
                .await;
        });
    });
    Ok(IsolatedProxy {
        base_url: format!("http://{address}"),
        worker: started.recv()?,
        shutdown,
        thread,
    })
}

/// Drives one forward on its own task, so the runtime alone decides when it is polled.
fn spawn_forward(
    client: &reqwest::Client,
    proxy: &str,
) -> tokio::task::JoinHandle<Result<(StatusCode, Bytes), reqwest::Error>> {
    let client = client.clone();
    let url = format!("{proxy}/v1/responses");
    tokio::spawn(async move {
        let response = client
            .post(url)
            .header("content-type", "application/json")
            .body(DOCUMENT_REQUEST)
            .send()
            .await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        Ok((status, bytes))
    })
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
