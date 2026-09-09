use std::{fmt, fs, io::ErrorKind, os::unix::fs::PermissionsExt as _, path::Path};

use crate::{
    Credential, Endpoint, IpcError, IpcLimits, IpcTransport, ServerConnection, SocketOwner,
    UnixEndpoint,
};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;

mod artifacts;
use artifacts::{
    SocketArtifacts, StagedSocket, create_marker, marker_path, prepare_socket_path,
    socket_identity, write_marker,
};

#[cfg(test)]
mod tests;

/// Authenticated Unix transport configuration with explicit ownership and frame bounds.
#[derive(Clone, Debug)]
pub struct UnixTransportConfig {
    binding: UnixBinding,
    credential: Credential,
    limits: IpcLimits,
}

/// Listener-only ownership paired with a public Unix client endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnixBinding {
    endpoint: UnixEndpoint,
    owner: SocketOwner,
}

impl UnixBinding {
    /// Associates an opaque cleanup owner with a public endpoint.
    #[must_use]
    pub const fn new(endpoint: UnixEndpoint, owner: SocketOwner) -> Self {
        Self { endpoint, owner }
    }
}

impl UnixTransportConfig {
    /// Creates configuration for a socket inside a private caller-owned directory.
    #[must_use]
    pub const fn authenticated(
        binding: UnixBinding,
        credential: Credential,
        limits: IpcLimits,
    ) -> Self {
        Self {
            binding,
            credential,
            limits,
        }
    }
}

/// Owned Unix listener whose filesystem artifacts are removed on drop.
pub struct UnixTransport {
    listener: UnixListener,
    artifacts: SocketArtifacts,
    config: UnixTransportConfig,
}

impl UnixTransport {
    /// Binds after proving the directory private and any same-owner socket stale.
    ///
    /// # Errors
    /// Refuses non-socket, foreign-owner, active, ambiguous, NUL, permissions, and I/O cases.
    pub fn bind(config: UnixTransportConfig) -> Result<Self, IpcError> {
        Self::bind_with_hook(config, |_| Ok(()))
    }

    fn bind_with_hook<Hook>(
        config: UnixTransportConfig,
        after_listener_created: Hook,
    ) -> Result<Self, IpcError>
    where
        Hook: FnOnce(&Path) -> Result<(), IpcError>,
    {
        ensure_private_parent(config.binding.endpoint.path())?;
        let marker = marker_path(config.binding.endpoint.path());
        prepare_socket_path(
            config.binding.endpoint.path(),
            &marker,
            config.binding.owner.bytes(),
        )?;
        ensure_socket_path_addressable(config.binding.endpoint.path())?;
        let mut artifacts =
            SocketArtifacts::new(config.binding.endpoint.path().to_path_buf(), marker);
        let (mut marker_file, marker_identity) = create_marker(artifacts.marker())?;
        artifacts.set_marker(marker_identity);
        write_marker(&mut marker_file, config.binding.owner.bytes())?;
        drop(marker_file);
        let staged = StagedSocket::create(config.binding.endpoint.path())?;
        let listener = UnixListener::bind(staged.path())?;
        let socket_identity = socket_identity(staged.path())?;
        fs::set_permissions(staged.path(), fs::Permissions::from_mode(0o600))?;
        after_listener_created(config.binding.endpoint.path())?;
        staged.publish(config.binding.endpoint.path(), socket_identity)?;
        artifacts.set_socket(socket_identity);
        Ok(Self {
            listener,
            artifacts,
            config,
        })
    }

    /// Removes this listener's filesystem artifacts after revalidating their identities.
    ///
    /// # Errors
    /// Returns identity or I/O evidence when cleanup cannot safely remove an artifact.
    pub fn close(mut self) -> Result<(), IpcError> {
        self.artifacts.cleanup()
    }
}

impl fmt::Debug for UnixTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnixTransport")
            .field("endpoint", &self.config.binding.endpoint)
            .field("credential", &"[REDACTED]")
            .field("limits", &self.config.limits)
            .finish_non_exhaustive()
    }
}

impl IpcTransport for UnixTransport {
    type AcceptedStream = UnixStream;

    fn endpoint(&self) -> Endpoint {
        Endpoint::Unix(self.config.binding.endpoint.clone())
    }

    async fn accept(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<ServerConnection<Self::AcceptedStream>, IpcError> {
        let (stream, _peer) = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(IpcError::Cancelled),
            result = self.listener.accept() => result.map_err(IpcError::from)?,
        };
        Ok(ServerConnection::new(
            stream,
            self.config.credential.clone(),
            self.config.limits,
        ))
    }
}

fn ensure_private_parent(socket: &Path) -> Result<(), IpcError> {
    let parent = socket.parent().ok_or_else(|| IpcError::Io {
        source: std::io::Error::new(ErrorKind::InvalidInput, "socket path has no parent"),
    })?;
    let metadata = fs::metadata(parent)?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(IpcError::InsecureSocketDirectory);
    }
    Ok(())
}

fn ensure_socket_path_addressable(socket: &Path) -> Result<(), IpcError> {
    match std::os::unix::net::UnixStream::connect(socket) {
        Ok(_active) => Err(IpcError::SocketAlreadyActive),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
