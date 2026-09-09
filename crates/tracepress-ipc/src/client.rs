use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::sync::CancellationToken;

use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows;

use crate::{ClientConnection, Credential, Endpoint, IpcError, IpcLimits};

trait AsyncIpcStream: AsyncRead + AsyncWrite + Send + Unpin {}

impl<Stream> AsyncIpcStream for Stream where Stream: AsyncRead + AsyncWrite + Send + Unpin {}

/// Type-erased local stream returned by the transport-neutral client connector.
pub struct ClientStream(Box<dyn AsyncIpcStream>);

impl ClientStream {
    fn new<Stream>(stream: Stream) -> Self
    where
        Stream: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        Self(Box::new(stream))
    }
}

impl fmt::Debug for ClientStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClientStream(..)")
    }
}

impl AsyncRead for ClientStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_read(context, buffer)
    }
}

impl AsyncWrite for ClientStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut *self.get_mut().0).poll_write(context, buffer)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut *self.get_mut().0).poll_flush(context)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut *self.get_mut().0).poll_shutdown(context)
    }
}

/// Authenticated connector constructed without listener ownership.
#[derive(Clone, Debug)]
pub struct IpcClient {
    endpoint: Endpoint,
    credential: Credential,
    limits: IpcLimits,
}

impl IpcClient {
    /// Creates a client from public connection material only.
    #[must_use]
    pub const fn authenticated(
        endpoint: Endpoint,
        credential: Credential,
        limits: IpcLimits,
    ) -> Self {
        Self {
            endpoint,
            credential,
            limits,
        }
    }

    /// Connects to the configured local endpoint with cancellation.
    ///
    /// # Errors
    /// Returns typed cancellation, unsupported-platform, or operating-system failures.
    pub async fn connect(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<ClientConnection<ClientStream>, IpcError> {
        let stream = match &self.endpoint {
            Endpoint::Tcp(endpoint) => {
                let stream = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => return Err(IpcError::Cancelled),
                    result = TcpStream::connect(endpoint.address()) => result?,
                };
                if !stream.peer_addr()?.ip().is_loopback() {
                    return Err(IpcError::TcpMustBeLoopback);
                }
                ClientStream::new(stream)
            }
            #[cfg(unix)]
            Endpoint::Unix(endpoint) => {
                let stream = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => return Err(IpcError::Cancelled),
                    result = UnixStream::connect(endpoint.path()) => result?,
                };
                ClientStream::new(stream)
            }
            #[cfg(not(unix))]
            Endpoint::Unix(_) => return Err(IpcError::UnsupportedEndpoint),
            #[cfg(windows)]
            Endpoint::NamedPipe(endpoint) => {
                let stream = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => return Err(IpcError::Cancelled),
                    result = async { windows::named_pipe::ClientOptions::new().open(endpoint.name()) } => result?,
                };
                ClientStream::new(stream)
            }
            #[cfg(not(windows))]
            Endpoint::NamedPipe(_) => return Err(IpcError::UnsupportedEndpoint),
        };
        Ok(ClientConnection::new(
            stream,
            self.credential.clone(),
            self.limits,
        ))
    }
}
