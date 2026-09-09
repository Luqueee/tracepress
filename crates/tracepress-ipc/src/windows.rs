use tokio::{
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

use crate::{
    Credential, Endpoint, IpcError, IpcLimits, IpcTransport, NamedPipeEndpoint, ServerConnection,
};

/// Windows named-pipe transport prepared behind the platform cfg boundary.
#[derive(Debug)]
pub struct NamedPipeTransport {
    endpoint: NamedPipeEndpoint,
    credential: Credential,
    limits: IpcLimits,
    next_server: Mutex<Option<NamedPipeServer>>,
}

impl NamedPipeTransport {
    /// Creates the first private named-pipe server instance.
    ///
    /// # Errors
    /// Returns a typed operating-system creation failure.
    pub fn bind(
        endpoint: NamedPipeEndpoint,
        credential: Credential,
        limits: IpcLimits,
    ) -> Result<Self, IpcError> {
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(endpoint.name())?;
        Ok(Self {
            endpoint,
            credential,
            limits,
            next_server: Mutex::new(Some(server)),
        })
    }
}

impl IpcTransport for NamedPipeTransport {
    type AcceptedStream = NamedPipeServer;

    fn endpoint(&self) -> Endpoint {
        Endpoint::NamedPipe(self.endpoint.clone())
    }

    async fn accept(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<ServerConnection<Self::AcceptedStream>, IpcError> {
        let server = {
            let mut slot = self.next_server.lock().await;
            let server = slot.take().ok_or(IpcError::SocketStateUnknown)?;
            *slot = Some(ServerOptions::new().create(self.endpoint.name())?);
            server
        };
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(IpcError::Cancelled),
            result = server.connect() => result.map_err(IpcError::from)?,
        }
        Ok(ServerConnection::new(
            server,
            self.credential.clone(),
            self.limits,
        ))
    }
}
