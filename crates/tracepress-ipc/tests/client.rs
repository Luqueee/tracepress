//! Listener-independent client and body-boundary behavior.

use tokio_util::sync::CancellationToken;
use tracepress_core::{
    MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes, RequestId, UuidV7Generator,
};
use tracepress_ipc::{
    Credential, IpcClient, IpcError, IpcLimits, IpcRequest, IpcResponse, IpcTransport,
    ResponseOutcome, TcpTransport, TcpTransportConfig,
};

#[cfg(unix)]
use tracepress_ipc::{Endpoint, UnixEndpoint};

fn limits(request: u64, response: u64) -> Result<IpcLimits, tracepress_core::LimitValueError> {
    Ok(IpcLimits::new(
        MaxIpcFrameBytes::new(4_096)?,
        MaxRequestBodyBytes::new(request)?,
        MaxResponseBodyBytes::new(response)?,
    ))
}

#[cfg(unix)]
#[test]
fn unix_client_configuration_requires_no_listener_owner() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let directory = tempfile::tempdir()?;
    let endpoint = UnixEndpoint::new(directory.path().join("ipc.sock"))?;

    // When
    let client = IpcClient::authenticated(
        Endpoint::Unix(endpoint),
        Credential::new([5; 32]),
        limits(1_024, 1_024)?,
    );

    // Then
    assert!(format!("{client:?}").contains("Unix"));
    Ok(())
}

#[tokio::test]
async fn independent_client_connects_from_endpoint_and_credentials()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let credential = Credential::new([6; 32]);
    let connection_limits = limits(1_024, 1_024)?;
    let transport = TcpTransport::bind(TcpTransportConfig::authenticated(
        credential.clone(),
        connection_limits,
    ))
    .await?;
    let client = IpcClient::authenticated(transport.endpoint(), credential, connection_limits);
    let cancellation = CancellationToken::new();

    // When
    let (server_result, client_result) = tokio::join!(
        transport.accept(&cancellation),
        client.connect(&cancellation)
    );

    // Then
    let _server_connection = server_result?;
    let _client_connection = client_result?;
    Ok(())
}

#[tokio::test]
async fn client_revalidates_request_body_against_connection_limit()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let credential = Credential::new([7; 32]);
    let connection_limits = limits(1, 1)?;
    let transport = TcpTransport::bind(TcpTransportConfig::authenticated(
        credential.clone(),
        connection_limits,
    ))
    .await?;
    let client = IpcClient::authenticated(transport.endpoint(), credential, connection_limits);
    let cancellation = CancellationToken::new();
    let request = IpcRequest::new(
        RequestId::generate(&UuidV7Generator::new()),
        vec![1, 2],
        MaxRequestBodyBytes::new(2)?,
    )?;
    let (server_result, client_result) = tokio::join!(
        transport.accept(&cancellation),
        client.connect(&cancellation)
    );
    let _server = server_result?;
    let mut connection = client_result?;

    // When
    let result = connection.send_request(&request, &cancellation).await;

    // Then
    assert!(matches!(
        result,
        Err(IpcError::RequestBodyTooLarge {
            observed: 2,
            maximum: 1
        })
    ));
    Ok(())
}

#[tokio::test]
async fn server_revalidates_response_body_against_connection_limit()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let credential = Credential::new([8; 32]);
    let connection_limits = limits(1, 1)?;
    let transport = TcpTransport::bind(TcpTransportConfig::authenticated(
        credential.clone(),
        connection_limits,
    ))
    .await?;
    let client = IpcClient::authenticated(transport.endpoint(), credential, connection_limits);
    let cancellation = CancellationToken::new();
    let response = IpcResponse::new(
        RequestId::generate(&UuidV7Generator::new()),
        ResponseOutcome::complete(vec![1, 2], MaxResponseBodyBytes::new(2)?)?,
    );
    let (server_result, client_result) = tokio::join!(
        transport.accept(&cancellation),
        client.connect(&cancellation)
    );
    let mut connection = server_result?;
    let _client = client_result?;

    // When
    let result = connection.send_response(&response, &cancellation).await;

    // Then
    assert!(matches!(
        result,
        Err(IpcError::ResponseBodyTooLarge {
            observed: 2,
            maximum: 1
        })
    ));
    Ok(())
}
