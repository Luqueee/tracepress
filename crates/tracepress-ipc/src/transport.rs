use std::future::Future;

use subtle::ConstantTimeEq as _;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    Credential, Endpoint, FramePart, IpcError, IpcLimits, IpcRequest, IpcResponse,
    auth::CREDENTIAL_BYTES,
    frame::{FrameConfig, ReadConfig, read_exact_cancelled, read_message, write_message},
};

const AUTH_MAGIC: [u8; 4] = *b"TPA1";
const AUTH_PREFACE_BYTES: usize = AUTH_MAGIC.len() + CREDENTIAL_BYTES;

/// A client-side authenticated protocol connection over any async byte stream.
pub struct ClientConnection<Stream> {
    stream: Stream,
    credential: Credential,
    limits: IpcLimits,
}

impl<Stream> ClientConnection<Stream>
where
    Stream: AsyncRead + AsyncWrite + Send + Unpin,
{
    pub(crate) const fn new(stream: Stream, credential: Credential, limits: IpcLimits) -> Self {
        Self {
            stream,
            credential,
            limits,
        }
    }

    /// Sends one authenticated request.
    ///
    /// # Errors
    /// Returns typed framing, cancellation, or I/O failures.
    pub async fn send_request(
        &mut self,
        request: &IpcRequest,
        cancellation: &CancellationToken,
    ) -> Result<(), IpcError> {
        request.validate_body(self.limits.maximum_request_body())?;
        let mut preface = Zeroizing::new([0_u8; AUTH_PREFACE_BYTES]);
        let magic = preface
            .get_mut(..AUTH_MAGIC.len())
            .ok_or(IpcError::FrameLengthUnrepresentable)?;
        magic.copy_from_slice(&AUTH_MAGIC);
        let credential = preface
            .get_mut(AUTH_MAGIC.len()..)
            .ok_or(IpcError::FrameLengthUnrepresentable)?;
        credential.copy_from_slice(self.credential.bytes());
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(IpcError::Cancelled),
            result = self.stream.write_all(&preface[..]) => result.map_err(IpcError::from)?,
        }
        write_message(
            &mut self.stream,
            request,
            FrameConfig::new(self.limits.maximum_frame(), cancellation),
        )
        .await
    }

    /// Receives one typed response.
    ///
    /// # Errors
    /// Returns typed framing, malformed-message, cancellation, or I/O failures.
    pub async fn receive_response(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<IpcResponse, IpcError> {
        let response: IpcResponse = read_message(
            &mut self.stream,
            FrameConfig::new(self.limits.maximum_frame(), cancellation),
        )
        .await?;
        response.validate_body(self.limits.maximum_response_body())?;
        Ok(response)
    }
}

impl<Stream> std::fmt::Debug for ClientConnection<Stream> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientConnection")
            .field("credential", &"[REDACTED]")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// A server-side protocol connection that authenticates before decoding a request.
pub struct ServerConnection<Stream> {
    stream: Stream,
    expected_credential: Credential,
    limits: IpcLimits,
}

impl<Stream> ServerConnection<Stream>
where
    Stream: AsyncRead + AsyncWrite + Send + Unpin,
{
    pub(crate) const fn new(
        stream: Stream,
        expected_credential: Credential,
        limits: IpcLimits,
    ) -> Self {
        Self {
            stream,
            expected_credential,
            limits,
        }
    }

    /// Authenticates and receives one typed request.
    ///
    /// # Errors
    /// Returns typed invalid-auth, framing, cancellation, or I/O failures.
    pub async fn receive_request(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<IpcRequest, IpcError> {
        let mut preface = Zeroizing::new([0_u8; AUTH_PREFACE_BYTES]);
        read_exact_cancelled(
            &mut self.stream,
            &mut preface[..],
            ReadConfig {
                part: FramePart::Authentication,
                cancellation,
            },
        )
        .await?;
        let magic = preface
            .get(..AUTH_MAGIC.len())
            .ok_or(IpcError::MalformedAuthentication)?;
        if magic != AUTH_MAGIC {
            return Err(IpcError::MalformedAuthentication);
        }
        let supplied = preface
            .get(AUTH_MAGIC.len()..)
            .and_then(|bytes| <&[u8; CREDENTIAL_BYTES]>::try_from(bytes).ok())
            .ok_or(IpcError::MalformedAuthentication)?;
        if !bool::from(supplied.ct_eq(self.expected_credential.bytes())) {
            return Err(IpcError::InvalidAuthentication);
        }
        let request: IpcRequest = read_message(
            &mut self.stream,
            FrameConfig::new(self.limits.maximum_frame(), cancellation),
        )
        .await?;
        request.validate_body(self.limits.maximum_request_body())?;
        Ok(request)
    }

    /// Sends one typed response with no authentication fields.
    ///
    /// # Errors
    /// Returns typed framing, cancellation, or I/O failures.
    pub async fn send_response(
        &mut self,
        response: &IpcResponse,
        cancellation: &CancellationToken,
    ) -> Result<(), IpcError> {
        response.validate_body(self.limits.maximum_response_body())?;
        write_message(
            &mut self.stream,
            response,
            FrameConfig::new(self.limits.maximum_frame(), cancellation),
        )
        .await
    }
}

impl<Stream> std::fmt::Debug for ServerConnection<Stream> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerConnection")
            .field("expected_credential", &"[REDACTED]")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// Listener and connector abstraction independent of protocol framing.
pub trait IpcTransport: Send + Sync {
    /// Stream returned from server-side acceptance.
    type AcceptedStream: AsyncRead + AsyncWrite + Unpin + Send;

    /// Returns this transport's concrete local endpoint.
    fn endpoint(&self) -> Endpoint;

    /// Accepts one connection or returns cancellation without detaching work.
    fn accept<'operation>(
        &'operation self,
        cancellation: &'operation CancellationToken,
    ) -> impl Future<Output = Result<ServerConnection<Self::AcceptedStream>, IpcError>> + Send + 'operation;
}
