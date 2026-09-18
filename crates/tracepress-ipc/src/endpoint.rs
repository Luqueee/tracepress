use std::{
    fmt,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use subtle::ConstantTimeEq;

use crate::IpcError;

#[allow(
    clippy::redundant_pub_crate,
    reason = "the private endpoint module shares this marker width with its Unix adapter"
)]
pub(crate) const SOCKET_OWNER_BYTES: usize = 16;

/// Opaque ownership token guarding Unix socket cleanup.
#[derive(Clone, Eq)]
pub struct SocketOwner([u8; SOCKET_OWNER_BYTES]);

impl SocketOwner {
    /// Creates an instance ownership token.
    #[must_use]
    pub const fn new(bytes: [u8; SOCKET_OWNER_BYTES]) -> Self {
        Self(bytes)
    }

    #[cfg(unix)]
    pub(crate) const fn bytes(&self) -> &[u8; SOCKET_OWNER_BYTES] {
        &self.0
    }
}

impl fmt::Debug for SocketOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SocketOwner([REDACTED])")
    }
}

impl PartialEq for SocketOwner {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.0.ct_eq(&other.0))
    }
}

/// Filesystem endpoint and ownership token for a Unix domain socket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnixEndpoint {
    path: PathBuf,
}

impl UnixEndpoint {
    /// Creates a socket endpoint beneath a caller-owned private directory.
    ///
    /// # Errors
    /// Returns [`IpcError::EndpointContainsNul`] when the path contains NUL.
    pub fn new(path: PathBuf) -> Result<Self, IpcError> {
        if path.to_string_lossy().contains('\0') {
            return Err(IpcError::EndpointContainsNul);
        }
        Ok(Self { path })
    }

    /// Returns the socket path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Validated loopback TCP endpoint assigned by an ephemeral bind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpEndpoint(SocketAddr);

impl TcpEndpoint {
    /// Creates a client endpoint from an assigned loopback address.
    ///
    /// # Errors
    /// Rejects non-loopback addresses and port zero.
    pub const fn new(address: SocketAddr) -> Result<Self, IpcError> {
        if !address.ip().is_loopback() {
            return Err(IpcError::TcpMustBeLoopback);
        }
        if address.port() == 0 {
            return Err(IpcError::TcpPortUnassigned);
        }
        Ok(Self(address))
    }

    pub(crate) const fn from_bound(address: SocketAddr) -> Result<Self, IpcError> {
        Self::new(address)
    }

    /// Returns the assigned loopback address and ephemeral port.
    #[must_use]
    pub const fn address(self) -> SocketAddr {
        self.0
    }
}

/// Windows named-pipe endpoint kept independent from protocol messages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedPipeEndpoint(String);

impl NamedPipeEndpoint {
    /// Creates a named-pipe endpoint from an instance-local pipe name.
    ///
    /// # Errors
    /// Returns [`IpcError::EndpointContainsNul`] when `name` contains NUL.
    pub fn new(name: &str) -> Result<Self, IpcError> {
        if name.contains('\0') {
            return Err(IpcError::EndpointContainsNul);
        }
        Ok(Self(format!(r"\\.\pipe\{name}")))
    }

    /// Returns the platform pipe name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// A local IPC endpoint without concrete stream types.
#[allow(
    clippy::exhaustive_enums,
    reason = "transport dispatch must handle each supported local endpoint explicitly"
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    /// Unix domain socket endpoint.
    Unix(UnixEndpoint),
    /// Windows named-pipe endpoint.
    NamedPipe(NamedPipeEndpoint),
    /// Authenticated loopback TCP fallback endpoint.
    Tcp(TcpEndpoint),
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use super::*;

    #[test]
    fn socket_owner_equality_works_without_debug_disclosure() {
        let owner = SocketOwner::new([7; SOCKET_OWNER_BYTES]);
        let same = SocketOwner::new([7; SOCKET_OWNER_BYTES]);
        let different = SocketOwner::new([8; SOCKET_OWNER_BYTES]);

        assert_eq!(owner, same);
        assert_ne!(owner, different);
        assert_eq!(format!("{owner:?}"), "SocketOwner([REDACTED])");
    }

    #[test]
    fn unix_endpoint_rejects_nul_and_preserves_valid_path() -> Result<(), Box<dyn std::error::Error>>
    {
        let valid_path = PathBuf::from("/tmp/tracepress.sock");
        let endpoint = UnixEndpoint::new(valid_path.clone())?;
        let with_nul = UnixEndpoint::new(PathBuf::from("/tmp/tracepress\0.sock"));

        assert_eq!(endpoint.path(), valid_path);
        assert!(matches!(with_nul, Err(IpcError::EndpointContainsNul)));
        Ok(())
    }

    #[test]
    fn tcp_endpoint_accepts_assigned_ipv4_and_ipv6_loopback_addresses()
    -> Result<(), Box<dyn std::error::Error>> {
        let ipv4 = SocketAddr::from(([127, 0, 0, 1], 4319));
        let ipv6 = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 4319));

        assert_eq!(TcpEndpoint::new(ipv4)?.address(), ipv4);
        assert_eq!(TcpEndpoint::new(ipv6)?.address(), ipv6);
        Ok(())
    }

    #[test]
    fn tcp_endpoint_rejects_non_loopback_and_unassigned_ports() {
        let non_loopback = SocketAddr::from(([192, 0, 2, 1], 4319));
        let unassigned = SocketAddr::from(([127, 0, 0, 1], 0));

        assert!(matches!(
            TcpEndpoint::new(non_loopback),
            Err(IpcError::TcpMustBeLoopback)
        ));
        assert!(matches!(
            TcpEndpoint::new(unassigned),
            Err(IpcError::TcpPortUnassigned)
        ));
    }

    #[test]
    fn named_pipe_endpoint_prefixes_name_and_rejects_nul() -> Result<(), Box<dyn std::error::Error>>
    {
        let endpoint = NamedPipeEndpoint::new("tracepress-test")?;

        assert_eq!(endpoint.name(), r"\\.\pipe\tracepress-test");
        assert!(matches!(
            NamedPipeEndpoint::new("tracepress\0test"),
            Err(IpcError::EndpointContainsNul)
        ));
        Ok(())
    }
}
