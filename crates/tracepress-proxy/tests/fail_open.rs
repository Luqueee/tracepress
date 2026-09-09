//! Auxiliary metadata failures must not change transparent forwarding.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::Request,
    http::StatusCode,
    response::Response,
    routing::post,
};
use futures_util::stream;
use tokio::net::TcpListener;
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes};
use tracepress_provider::{ProviderEndpoint, RequestObservation, ResponseObservation};
use tracepress_proxy::{
    ForwardId, ForwardMetadata, MetadataSink, MetadataSinkError, ObservationSinkError,
    ProviderObservationSink, ProxyConfig, TransparentProxy, TransportFailure,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Default)]
struct FailingSink;

#[derive(Clone, Default)]
struct RecordingObservationSink {
    requests: Arc<Mutex<Vec<RequestObservation>>>,
    responses: Arc<Mutex<Vec<ResponseObservation>>>,
    failures: Arc<Mutex<Vec<TransportFailure>>>,
}

impl ProviderObservationSink for RecordingObservationSink {
    fn try_record_request(
        &self,
        _forward: ForwardId,
        observation: RequestObservation,
    ) -> Result<(), ObservationSinkError> {
        self.requests
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push(observation);
        Ok(())
    }

    fn try_record_response(
        &self,
        _forward: ForwardId,
        observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        self.responses
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push(observation);
        Ok(())
    }

    fn try_record_transport_failure(
        &self,
        _forward: ForwardId,
        failure: TransportFailure,
    ) -> Result<(), ObservationSinkError> {
        self.failures
            .lock()
            .map_err(|_error| ObservationSinkError::rejected())?
            .push(failure);
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ObservationSinkBehavior {
    Failing,
    Slow,
}

struct BehaviorObservationSink {
    behavior: ObservationSinkBehavior,
}

impl ProviderObservationSink for BehaviorObservationSink {
    fn try_record_request(
        &self,
        _forward: ForwardId,
        _observation: RequestObservation,
    ) -> Result<(), ObservationSinkError> {
        self.apply()
    }

    fn try_record_response(
        &self,
        _forward: ForwardId,
        _observation: ResponseObservation,
    ) -> Result<(), ObservationSinkError> {
        self.apply()
    }

    fn try_record_transport_failure(
        &self,
        _forward: ForwardId,
        _failure: TransportFailure,
    ) -> Result<(), ObservationSinkError> {
        self.apply()
    }
}

impl BehaviorObservationSink {
    fn apply(&self) -> Result<(), ObservationSinkError> {
        if matches!(self.behavior, ObservationSinkBehavior::Slow) {
            std::thread::sleep(Duration::from_secs(2));
        }
        Err(ObservationSinkError::rejected())
    }
}
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

#[tokio::test]
async fn responses_observations_are_delivered_as_a_side_channel() -> TestResult {
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_address = upstream_listener.local_addr()?;
    let upstream_task = tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/responses",
            post(|_request: Request| async {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":"resp_1","status":"completed"}"#))
                    .unwrap_or_else(|_error| Response::new(Body::empty()))
            }),
        );
        let _result = axum::serve(upstream_listener, app).await;
    });
    let endpoint = ProviderEndpoint::new(&format!("http://{upstream_address}/v1/responses"))?;
    let sink = Arc::new(RecordingObservationSink::default());
    let proxy = TransparentProxy::new(ProxyConfig::new(
        endpoint,
        MaxRequestBodyBytes::new(4_096)?,
        MaxResponseBodyBytes::new(4_096)?,
    ))?
    .with_observation_sink(Arc::<RecordingObservationSink>::clone(&sink));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_address = proxy_listener.local_addr()?;
    let proxy_task = tokio::spawn(async move {
        let _result = axum::serve(proxy_listener, proxy.router()).await;
    });

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_address}/v1/responses"))
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-5","stream":false}"#)
        .send()
        .await?;
    let status = response.status();
    let body = response.bytes().await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), br#"{"id":"resp_1","status":"completed"}"#);

    for _attempt in 0..100 {
        let request_count = sink
            .requests
            .lock()
            .map_err(|_error| "request observation lock poisoned")?
            .len();
        let response_count = sink
            .responses
            .lock()
            .map_err(|_error| "response observation lock poisoned")?
            .len();
        if request_count == 1 && response_count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        sink.requests
            .lock()
            .map_err(|_error| "request observation lock poisoned")?
            .len(),
        1
    );
    assert_eq!(
        sink.responses
            .lock()
            .map_err(|_error| "response observation lock poisoned")?
            .len(),
        1
    );
    assert!(
        sink.failures
            .lock()
            .map_err(|_error| "transport failure lock poisoned")?
            .is_empty(),
        "an answered forward must not be reported as a transport failure"
    );
    proxy_task.abort();
    upstream_task.abort();
    Ok(())
}

#[tokio::test]
async fn failing_and_slow_observation_sinks_are_fail_open_for_streams() -> TestResult {
    let chunks = vec![
        Bytes::from_static(b"event: response.created\ndata: {}\n\n"),
        Bytes::from_static(b"event: response.completed\ndata: {}\n\n\0\xff"),
    ];
    for (behavior, timeout) in [
        (ObservationSinkBehavior::Failing, Duration::from_secs(2)),
        (ObservationSinkBehavior::Slow, Duration::from_millis(500)),
    ] {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
        let upstream_address = upstream_listener.local_addr()?;
        let upstream_chunks = chunks.clone();
        let upstream_task = tokio::spawn(async move {
            let app = Router::new().route(
                "/v1/responses",
                post(move |_request: Request| {
                    let chunks = upstream_chunks.clone();
                    async move {
                        let body = stream::iter(
                            chunks
                                .into_iter()
                                .map(Ok::<Bytes, std::convert::Infallible>),
                        );
                        Response::builder()
                            .status(StatusCode::TOO_MANY_REQUESTS)
                            .header("content-type", "text/event-stream")
                            .body(Body::from_stream(body))
                            .unwrap_or_else(|_error| Response::new(Body::empty()))
                    }
                }),
            );
            let _result = axum::serve(upstream_listener, app).await;
        });
        let endpoint = ProviderEndpoint::new(&format!("http://{upstream_address}/v1/responses"))?;
        let proxy = TransparentProxy::new(ProxyConfig::new(
            endpoint,
            MaxRequestBodyBytes::new(4_096)?,
            MaxResponseBodyBytes::new(4_096)?,
        ))?
        .with_observation_sink(Arc::new(BehaviorObservationSink { behavior }));
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await?;
        let proxy_address = proxy_listener.local_addr()?;
        let proxy_task = tokio::spawn(async move {
            let _result = axum::serve(proxy_listener, proxy.router()).await;
        });

        let response = tokio::time::timeout(
            timeout,
            reqwest::Client::new()
                .post(format!("http://{proxy_address}/v1/responses"))
                .header("content-type", "application/json")
                .body(r#"{"model":"gpt-5","stream":true}"#)
                .send(),
        )
        .await??;
        let status = response.status();
        let body = response.bytes().await?;
        let expected = chunks
            .iter()
            .flat_map(|chunk| chunk.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.as_ref(), expected.as_slice());
        proxy_task.abort();
        upstream_task.abort();
    }
    Ok(())
}
