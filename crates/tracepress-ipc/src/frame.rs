use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio_util::sync::CancellationToken;
use tracepress_core::{ByteDecision, MaxIpcFrameBytes, classify_ipc_frame};

use crate::{FramePart, IpcError};

mod serialize;

use self::serialize::serialize_bounded;

const FRAME_HEADER_BYTES: usize = 4;

/// Core frame limit and cancellation context for one I/O operation.
#[derive(Clone, Copy, Debug)]
pub struct FrameConfig<'operation> {
    maximum: MaxIpcFrameBytes,
    cancellation: &'operation CancellationToken,
}

impl<'operation> FrameConfig<'operation> {
    /// Creates a bounded frame operation context.
    #[must_use]
    pub const fn new(
        maximum: MaxIpcFrameBytes,
        cancellation: &'operation CancellationToken,
    ) -> Self {
        Self {
            maximum,
            cancellation,
        }
    }
}

#[derive(Clone, Copy)]
#[allow(
    clippy::redundant_pub_crate,
    reason = "the private framing module supplies exact-read context to the transport sibling"
)]
pub(crate) struct ReadConfig<'operation> {
    pub(crate) part: FramePart,
    pub(crate) cancellation: &'operation CancellationToken,
}

#[allow(
    clippy::redundant_pub_crate,
    reason = "the private framing module shares exact reads with the transport sibling"
)]
pub(crate) async fn read_exact_cancelled<Reader>(
    reader: &mut Reader,
    output: &mut [u8],
    config: ReadConfig<'_>,
) -> Result<(), IpcError>
where
    Reader: AsyncRead + Send + Unpin,
{
    let expected = output.len();
    let mut received = 0_usize;
    while received < expected {
        let remaining = output
            .get_mut(received..)
            .ok_or(IpcError::FrameLengthUnrepresentable)?;
        let read = tokio::select! {
            biased;
            () = config.cancellation.cancelled() => return Err(IpcError::Cancelled),
            result = reader.read(remaining) => result.map_err(IpcError::from)?,
        };
        if read == 0 {
            return Err(IpcError::Truncated {
                part: config.part,
                expected,
                received,
            });
        }
        received = received
            .checked_add(read)
            .ok_or(IpcError::FrameLengthUnrepresentable)?;
    }
    Ok(())
}

/// Reads one length-prefixed frame, rejecting its declaration before allocation.
///
/// # Errors
/// Returns typed cancellation, truncation, size, allocation, or I/O failures.
pub async fn read_frame<Reader>(
    reader: &mut Reader,
    config: FrameConfig<'_>,
) -> Result<Vec<u8>, IpcError>
where
    Reader: AsyncRead + Send + Unpin,
{
    let mut header = [0_u8; FRAME_HEADER_BYTES];
    read_exact_cancelled(
        reader,
        &mut header,
        ReadConfig {
            part: FramePart::Header,
            cancellation: config.cancellation,
        },
    )
    .await?;
    let declared = u64::from(u32::from_be_bytes(header));
    if declared > config.maximum.get() {
        return Err(IpcError::FrameTooLarge {
            declared,
            maximum: config.maximum.get(),
        });
    }
    let payload_length =
        usize::try_from(declared).map_err(|_error| IpcError::FrameLengthUnrepresentable)?;
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(payload_length)
        .map_err(|_error| IpcError::AllocationFailed {
            requested: payload_length,
        })?;
    payload.resize(payload_length, 0);
    read_exact_cancelled(
        reader,
        &mut payload,
        ReadConfig {
            part: FramePart::Payload,
            cancellation: config.cancellation,
        },
    )
    .await?;
    match classify_ipc_frame(&payload, declared, config.maximum)
        .map_err(|_error| IpcError::FrameLengthUnrepresentable)?
    {
        ByteDecision::WithinLimit { bytes: _ } => Ok(payload),
        ByteDecision::Reject { violation }
        | ByteDecision::Bypass {
            bytes: _,
            violation,
        } => Err(IpcError::FrameTooLarge {
            declared: violation.observed,
            maximum: violation.maximum,
        }),
        ByteDecision::TruncateSafe { bytes: _, metadata } => Err(IpcError::FrameTooLarge {
            declared: metadata.original_bytes,
            maximum: config.maximum.get(),
        }),
    }
}

/// Writes one bounded length-prefixed frame.
///
/// # Errors
/// Returns typed cancellation, size, conversion, or I/O failures.
pub async fn write_frame<Writer>(
    writer: &mut Writer,
    payload: &[u8],
    config: FrameConfig<'_>,
) -> Result<(), IpcError>
where
    Writer: AsyncWrite + Send + Unpin,
{
    let declared =
        u64::try_from(payload.len()).map_err(|_error| IpcError::FrameLengthUnrepresentable)?;
    match classify_ipc_frame(payload, declared, config.maximum)
        .map_err(|_error| IpcError::FrameLengthUnrepresentable)?
    {
        ByteDecision::WithinLimit { bytes: _ } => {}
        ByteDecision::Reject { violation }
        | ByteDecision::Bypass {
            bytes: _,
            violation,
        } => {
            return Err(IpcError::FrameTooLarge {
                declared: violation.observed,
                maximum: violation.maximum,
            });
        }
        ByteDecision::TruncateSafe { bytes: _, metadata } => {
            return Err(IpcError::FrameTooLarge {
                declared: metadata.original_bytes,
                maximum: config.maximum.get(),
            });
        }
    }
    let wire_length =
        u32::try_from(declared).map_err(|_error| IpcError::FrameLengthUnrepresentable)?;
    let header = wire_length.to_be_bytes();
    tokio::select! {
        biased;
        () = config.cancellation.cancelled() => Err(IpcError::Cancelled),
        result = async {
            writer.write_all(&header).await?;
            writer.write_all(payload).await?;
            writer.flush().await
        } => result.map_err(IpcError::from),
    }
}

/// Reads and deserializes one typed frame.
///
/// # Errors
/// Returns framing failures or [`IpcError::MalformedMessage`].
pub async fn read_message<Reader, Message>(
    reader: &mut Reader,
    config: FrameConfig<'_>,
) -> Result<Message, IpcError>
where
    Reader: AsyncRead + Send + Unpin,
    Message: DeserializeOwned + Send,
{
    let payload = read_frame(reader, config).await?;
    serde_json::from_slice(&payload).map_err(|source| IpcError::MalformedMessage { source })
}

/// Serializes and writes one typed frame.
///
/// # Errors
/// Returns serialization or framing failures.
pub async fn write_message<Writer, Message>(
    writer: &mut Writer,
    message: &Message,
    config: FrameConfig<'_>,
) -> Result<(), IpcError>
where
    Writer: AsyncWrite + Send + Unpin,
    Message: Serialize + Sync,
{
    let payload = serialize_bounded(message, config.maximum)?;
    write_frame(writer, &payload, config).await
}
