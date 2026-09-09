//! Authenticated loopback TCP behavior through ephemeral endpoints.

use std::sync::Arc;

use tokio::{sync::Barrier, task::JoinSet};
use tokio_util::sync::CancellationToken;
use tracepress_core::{
    MaxIpcFrameBytes, MaxIpcQueueItems, MaxRequestBodyBytes, MaxResponseBodyBytes, RequestId,
    UuidV7Generator,
};
use tracepress_ipc::{
    Credential, Endpoint, IpcClient, IpcError, IpcLimits, IpcRequest, IpcResponse, IpcTransport,
    ResponseOutcome, TcpTransport, TcpTransportConfig, bounded_connections,
};

const fn frame_limit() -> Result<MaxIpcFrameBytes, tracepress_core::LimitValueError> {
    MaxIpcFrameBytes::new(4_096)
}

fn limits() -> Result<IpcLimits, tracepress_core::LimitValueError> {
    Ok(IpcLimits::new(
        frame_limit()?,
        MaxRequestBodyBytes::new(1_024)?,
        MaxResponseBodyBytes::new(1_024)?,
    ))
}

#[tokio::test]
async fn authenticated_round_trip_uses_ephemeral_loopback_tcp()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let transport = TcpTransport::bind(TcpTransportConfig::authenticated(
        Credential::new([7; 32]),
        limits()?,
    ))
    .await?;
    let Endpoint::Tcp(endpoint) = transport.endpoint() else {
        return Err("TCP transport returned another endpoint kind".into());
    };
    assert!(endpoint.address().ip().is_loopback());
    assert_ne!(endpoint.address().port(), 0);
    let request_id = RequestId::generate(&UuidV7Generator::new());
    let request = IpcRequest::new(request_id, vec![0, 2, 0, 4], MaxRequestBodyBytes::new(4)?)?;
    let response_limit = MaxResponseBodyBytes::new(4)?;
    let cancellation = CancellationToken::new();
    let client =
        IpcClient::authenticated(transport.endpoint(), Credential::new([7; 32]), limits()?);

    // When
    let server = async {
        let mut connection = transport.accept(&cancellation).await?;
        let received = connection.receive_request(&cancellation).await?;
        connection
            .send_response(
                &IpcResponse::new(
                    received.request_id(),
                    ResponseOutcome::complete(received.body().to_vec(), response_limit)?,
                ),
                &cancellation,
            )
            .await
    };
    let client = async {
        let mut connection = client.connect(&cancellation).await?;
        connection.send_request(&request, &cancellation).await?;
        connection.receive_response(&cancellation).await
    };
    let (server_result, client_result) = tokio::join!(server, client);
    server_result?;
    let response = client_result?;

    // Then
    assert_eq!(response.request_id(), request_id);
    assert_eq!(
        response.outcome(),
        &ResponseOutcome::complete(vec![0, 2, 0, 4], MaxResponseBodyBytes::new(4)?)?
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_clients_round_trip_through_core_bounded_queue()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    const CLIENTS: usize = 4;
    let transport = Arc::new(
        TcpTransport::bind(TcpTransportConfig::authenticated(
            Credential::new([10; 32]),
            limits()?,
        ))
        .await?,
    );
    let queue_limit = MaxIpcQueueItems::new(2)?;
    let (sender, mut receiver) = bounded_connections(queue_limit)?;
    let cancellation = CancellationToken::new();
    let barrier = Arc::new(Barrier::new(CLIENTS + 1));
    let request_limit = MaxRequestBodyBytes::new(2)?;
    let response_limit = MaxResponseBodyBytes::new(2)?;
    let endpoint = transport.endpoint();
    let mut clients = JoinSet::new();
    for body in 0_u8..4_u8 {
        let client =
            IpcClient::authenticated(endpoint.clone(), Credential::new([10; 32]), limits()?);
        let client_barrier = Arc::clone(&barrier);
        let client_cancellation = cancellation.clone();
        let _client_abort_handle = clients.spawn(async move {
            let _barrier_result = client_barrier.wait().await;
            let request = IpcRequest::new(
                RequestId::generate(&UuidV7Generator::new()),
                vec![body, 0],
                request_limit,
            )?;
            let mut connection = client.connect(&client_cancellation).await?;
            connection
                .send_request(&request, &client_cancellation)
                .await?;
            connection.receive_response(&client_cancellation).await
        });
    }
    let _barrier_result = barrier.wait().await;

    // When
    let acceptor = async {
        for _accepted in 0..CLIENTS {
            let connection = transport.accept(&cancellation).await?;
            sender.send(connection, &cancellation).await?;
        }
        drop(sender);
        Ok::<(), IpcError>(())
    };
    let worker = async {
        let mut handlers = JoinSet::new();
        while let Some(mut connection) = receiver.receive().await {
            let handler_cancellation = cancellation.clone();
            let _handler_abort_handle = handlers.spawn(async move {
                let request = connection.receive_request(&handler_cancellation).await?;
                connection
                    .send_response(
                        &IpcResponse::new(
                            request.request_id(),
                            ResponseOutcome::complete(request.body().to_vec(), response_limit)?,
                        ),
                        &handler_cancellation,
                    )
                    .await
            });
        }
        while let Some(joined) = handlers.join_next().await {
            joined??;
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    let (accept_result, worker_result) = tokio::join!(acceptor, worker);
    accept_result?;
    worker_result?;
    let mut responses = Vec::new();
    while let Some(joined) = clients.join_next().await {
        responses.push(joined??);
    }

    // Then
    assert_eq!(responses.len(), CLIENTS);
    assert!(responses.iter().all(|response| matches!(
        response.outcome(),
        ResponseOutcome::Complete { body } if body.len() == 2 && body.get(1) == Some(&0)
    )));
    Ok(())
}
