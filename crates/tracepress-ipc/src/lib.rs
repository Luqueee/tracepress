//! Transport-neutral, authenticated local IPC for Tracepress.

pub(crate) mod auth;
mod client;
pub(crate) mod endpoint;
mod error;
pub(crate) mod frame;
mod limits;
mod protocol;
mod queue;
mod tcp;
mod transport;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub use auth::Credential;
pub use client::{ClientStream, IpcClient};
pub use endpoint::{Endpoint, NamedPipeEndpoint, SocketOwner, TcpEndpoint, UnixEndpoint};
pub use error::{FramePart, IpcError};
pub use frame::{FrameConfig, read_frame, read_message, write_frame, write_message};
pub use limits::IpcLimits;
pub use protocol::{IpcRequest, IpcResponse, ResponseOutcome};
pub use queue::{ConnectionReceiver, ConnectionSender, bounded_connections};
pub use tcp::{TcpTransport, TcpTransportConfig};
pub use transport::{ClientConnection, IpcTransport, ServerConnection};
#[cfg(unix)]
pub use unix::{UnixBinding, UnixTransport, UnixTransportConfig};
#[cfg(windows)]
pub use windows::NamedPipeTransport;
