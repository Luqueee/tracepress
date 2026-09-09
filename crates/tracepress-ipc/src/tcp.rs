use std::{fmt, net::Ipv4Addr};

use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::{
    Credential, Endpoint, IpcError, IpcLimits, IpcTransport, ServerConnection, TcpEndpoint,
};

/// Explicit authenticated configuration for loopback TCP fallback.
#[derive(Clone, Debug)]
pub struct TcpTransportConfig {
    credential: Credential,
    limits: IpcLimits,
}

impl TcpTransportConfig {
    /// Enables TCP fallback with mandatory authentication and a core frame bound.
    #[must_use]
    pub const fn authenticated(credential: Credential, limits: IpcLimits) -> Self {
        Self { credential, limits }
    }
}

/// Authenticated listener bound only to an operating-system-assigned loopback port.
pub struct TcpTransport {
    listener: TcpListener,
    endpoint: TcpEndpoint,
    config: TcpTransportConfig,
}

impl TcpTransport {
    /// Binds to `127.0.0.1:0`; no unauthenticated or fixed-port constructor exists.
    ///
    /// # Errors
    /// Returns typed bind or endpoint validation failures.
    pub async fn bind(config: TcpTransportConfig) -> Result<Self, IpcError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let endpoint = TcpEndpoint::from_bound(listener.local_addr()?)?;
        Ok(Self {
            listener,
            endpoint,
            config,
        })
    }
}

impl fmt::Debug for TcpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TcpTransport")
            .field("endpoint", &self.endpoint)
            .field("credential", &"[REDACTED]")
            .field("limits", &self.config.limits)
            .finish_non_exhaustive()
    }
}

impl IpcTransport for TcpTransport {
    type AcceptedStream = TcpStream;

    fn endpoint(&self) -> Endpoint {
        Endpoint::Tcp(self.endpoint)
    }

    async fn accept(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<ServerConnection<Self::AcceptedStream>, IpcError> {
        let (stream, peer) = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(IpcError::Cancelled),
            result = self.listener.accept() => result.map_err(IpcError::from)?,
        };
        if !peer.ip().is_loopback() {
            return Err(IpcError::TcpMustBeLoopback);
        }
        Ok(ServerConnection::new(
            stream,
            self.config.credential.clone(),
            self.config.limits,
        ))
    }
}
