//! Authentication failure and truncation behavior.

use tokio::{io::AsyncWriteExt as _, net::TcpStream};
use tokio_util::sync::CancellationToken;
use tracepress_core::{
    MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes, RequestId, UuidV7Generator,
};
use tracepress_ipc::{
    Credential, Endpoint, FramePart, IpcClient, IpcError, IpcLimits, IpcRequest, IpcTransport,
    TcpTransport, TcpTransportConfig,
};

fn limits() -> Result<IpcLimits, tracepress_core::LimitValueError> {
    Ok(IpcLimits::new(
        MaxIpcFrameBytes::new(4_096)?,
        MaxRequestBodyBytes::new(1_024)?,
        MaxResponseBodyBytes::new(1_024)?,
    ))
}

#[tokio::test]
async fn invalid_authentication_is_typed_and_secret_safe() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let transport = TcpTransport::bind(TcpTransportConfig::authenticated(
        Credential::new([8; 32]),
        limits()?,
    ))
    .await?;
    let client =
        IpcClient::authenticated(transport.endpoint(), Credential::new([9; 32]), limits()?);
    let cancellation = CancellationToken::new();
    let request = IpcRequest::new(
        RequestId::generate(&UuidV7Generator::new()),
        vec![1],
        MaxRequestBodyBytes::new(1)?,
    )?;

    // When
    let server = async {
        let mut connection = transport.accept(&cancellation).await?;
        connection.receive_request(&cancellation).await
    };
    let client = async {
        let mut connection = client.connect(&cancellation).await?;
        connection.send_request(&request, &cancellation).await
    };
    let (server_result, client_result) = tokio::join!(server, client);
    client_result?;
    let Err(error) = server_result else {
        return Err("wrong credential was accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::InvalidAuthentication));
    let rendered = format!("{error:?}");
    assert!(!rendered.contains("0808"));
    assert!(!rendered.contains("0909"));
    Ok(())
}

#[tokio::test]
async fn malformed_authentication_preface_is_rejected_before_message_decode()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let transport = TcpTransport::bind(TcpTransportConfig::authenticated(
        Credential::new([11; 32]),
        limits()?,
    ))
    .await?;
    let Endpoint::Tcp(endpoint) = transport.endpoint() else {
        return Err("TCP transport returned another endpoint kind".into());
    };
    let cancellation = CancellationToken::new();

    // When
    let server = async {
        let mut connection = transport.accept(&cancellation).await?;
        connection.receive_request(&cancellation).await
    };
    let client = async {
        let mut stream = TcpStream::connect(endpoint.address()).await?;
        stream.write_all(&[0_u8; 36]).await
    };
    let (server_result, client_result) = tokio::join!(server, client);
    client_result?;
    let Err(error) = server_result else {
        return Err("malformed authentication preface was accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::MalformedAuthentication));
    Ok(())
}

#[tokio::test]
async fn truncated_authentication_preface_reports_exact_progress()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let transport = TcpTransport::bind(TcpTransportConfig::authenticated(
        Credential::new([12; 32]),
        limits()?,
    ))
    .await?;
    let Endpoint::Tcp(endpoint) = transport.endpoint() else {
        return Err("TCP transport returned another endpoint kind".into());
    };
    let cancellation = CancellationToken::new();

    // When
    let server = async {
        let mut connection = transport.accept(&cancellation).await?;
        connection.receive_request(&cancellation).await
    };
    let client = async {
        let mut stream = TcpStream::connect(endpoint.address()).await?;
        stream.write_all(&[0_u8; 7]).await?;
        stream.shutdown().await
    };
    let (server_result, client_result) = tokio::join!(server, client);
    client_result?;
    let Err(error) = server_result else {
        return Err("truncated authentication preface was accepted".into());
    };

    // Then
    assert!(matches!(
        error,
        IpcError::Truncated {
            part: FramePart::Authentication,
            expected: 36,
            received: 7
        }
    ));
    Ok(())
}
