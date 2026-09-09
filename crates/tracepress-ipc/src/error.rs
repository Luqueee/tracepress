use std::io;

use thiserror::Error;

/// The frame region in which an exact read ended early.
#[allow(
    clippy::exhaustive_enums,
    reason = "framing has a fixed header, payload, and authentication preface"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePart {
    /// Four-byte length header.
    Header,
    /// Declared frame payload.
    Payload,
    /// Authentication preface.
    Authentication,
}

/// Typed failures at the local IPC boundary.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IpcError {
    /// The operation was cancelled and its connection must be discarded.
    #[error("IPC operation cancelled")]
    Cancelled,
    /// An exact frame read ended before the declared boundary.
    #[error("truncated {part:?}: expected {expected} bytes, received {received}")]
    Truncated {
        /// The incomplete frame region.
        part: FramePart,
        /// Exact bytes required by the frame.
        expected: usize,
        /// Exact bytes received before EOF.
        received: usize,
    },
    /// A frame declaration exceeded the core maximum before allocation.
    #[error("IPC frame declares {declared} bytes above maximum {maximum}")]
    FrameTooLarge {
        /// Untrusted declared payload length.
        declared: u64,
        /// Configured core frame maximum.
        maximum: u64,
    },
    /// A frame length cannot be represented on this platform or wire format.
    #[error("IPC frame length is not representable")]
    FrameLengthUnrepresentable,
    /// Payload allocation failed within the configured bound.
    #[error("IPC frame allocation failed for {requested} bytes")]
    AllocationFailed {
        /// Requested bounded allocation.
        requested: usize,
    },
    /// An opaque request body exceeded its configured core limit.
    #[error("IPC request body has {observed} bytes above maximum {maximum}")]
    RequestBodyTooLarge {
        /// Exact request body length.
        observed: u64,
        /// Configured request body maximum.
        maximum: u64,
    },
    /// An opaque response body exceeded its configured core limit.
    #[error("IPC response body has {observed} bytes above maximum {maximum}")]
    ResponseBodyTooLarge {
        /// Exact response body length.
        observed: u64,
        /// Configured response body maximum.
        maximum: u64,
    },
    /// A typed message was malformed.
    #[error("malformed IPC message")]
    MalformedMessage {
        /// Serialization boundary failure without input bytes in its display.
        #[source]
        source: serde_json::Error,
    },
    /// The authentication preface had the wrong protocol marker.
    #[error("malformed IPC authentication preface")]
    MalformedAuthentication,
    /// Authentication failed without revealing supplied or expected bytes.
    #[error("invalid IPC authentication")]
    InvalidAuthentication,
    /// A TCP endpoint was not loopback-only.
    #[error("TCP IPC endpoint must use a loopback address")]
    TcpMustBeLoopback,
    /// A connected TCP endpoint did not have an assigned ephemeral port.
    #[error("TCP IPC endpoint must have an assigned ephemeral port")]
    TcpPortUnassigned,
    /// An endpoint name contained a NUL byte rejected by the operating system.
    #[error("IPC endpoint contains a NUL byte")]
    EndpointContainsNul,
    /// A bounded queue capacity cannot fit the platform channel API.
    #[error("IPC queue capacity is not representable on this platform")]
    QueueCapacityUnrepresentable,
    /// Deterministic admission rejected an item because the bounded queue is full.
    #[error("IPC connection queue is full at {maximum} items")]
    QueueFull {
        /// Configured core queue maximum.
        maximum: u64,
    },
    /// The bounded queue receiver was closed.
    #[error("IPC connection queue is closed")]
    QueueClosed,
    /// A Unix socket directory allowed group or other access.
    #[error("Unix socket directory is not private")]
    InsecureSocketDirectory,
    /// A Unix socket path was occupied by a non-socket filesystem entry.
    #[error("Unix socket path is occupied by a non-socket entry")]
    SocketPathNotSocket,
    /// Existing Unix socket artifacts did not carry this instance's owner token.
    #[error("Unix socket path is not owned by this instance")]
    SocketNotOwned,
    /// A Unix owner marker was not a no-follow regular file.
    #[error("Unix socket owner marker is invalid")]
    SocketMarkerInvalid,
    /// A Unix socket artifact changed identity before it could be removed.
    #[error(
        "Unix {artifact} changed identity: expected dev={expected_device} ino={expected_inode}, actual dev={actual_device} ino={actual_inode}"
    )]
    SocketArtifactChanged {
        /// Filesystem artifact kind, either `socket` or `owner marker`.
        artifact: &'static str,
        /// Device captured when the artifact was created.
        expected_device: u64,
        /// Inode captured when the artifact was created.
        expected_inode: u64,
        /// Device observed during cleanup.
        actual_device: u64,
        /// Inode observed during cleanup.
        actual_inode: u64,
    },
    /// An owned Unix socket is still accepting connections.
    #[error("Unix socket owned by this instance is already active")]
    SocketAlreadyActive,
    /// The state of an existing owned Unix socket could not be proven stale.
    #[error("Unix socket stale state could not be proven")]
    SocketStateUnknown,
    /// The selected endpoint is unavailable on this platform or transport.
    #[error("IPC endpoint is unsupported by this transport")]
    UnsupportedEndpoint,
    /// Operating-system I/O failed without embedding secret material.
    #[error("IPC I/O failed")]
    Io {
        /// Underlying operating-system error.
        #[source]
        source: io::Error,
    },
}

impl From<io::Error> for IpcError {
    fn from(source: io::Error) -> Self {
        Self::Io { source }
    }
}
