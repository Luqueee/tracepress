use std::io;

use serde::Serialize;
use tracepress_core::MaxIpcFrameBytes;

use crate::IpcError;

#[derive(Debug)]
enum WriteFailure {
    Limit { observed: usize },
    Allocation { requested: usize },
}

#[derive(Debug)]
struct BoundedOutput {
    bytes: Vec<u8>,
    maximum: usize,
    failure: Option<WriteFailure>,
}

impl BoundedOutput {
    const fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
            failure: None,
        }
    }
}

impl io::Write for BoundedOutput {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let observed = self
            .bytes
            .len()
            .checked_add(input.len())
            .ok_or_else(|| io::Error::other("serialized IPC message length overflow"))?;
        if observed > self.maximum {
            self.failure = Some(WriteFailure::Limit { observed });
            return Err(io::Error::other(
                "serialized IPC message exceeds frame limit",
            ));
        }
        if self.bytes.try_reserve_exact(input.len()).is_err() {
            self.failure = Some(WriteFailure::Allocation {
                requested: observed,
            });
            return Err(io::Error::other(
                "bounded IPC serialization allocation failed",
            ));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn serialize_bounded<Message>(
    message: &Message,
    maximum: MaxIpcFrameBytes,
) -> Result<Vec<u8>, IpcError>
where
    Message: Serialize,
{
    let wire_maximum = maximum.get().min(u64::from(u32::MAX));
    let capacity =
        usize::try_from(wire_maximum).map_err(|_error| IpcError::FrameLengthUnrepresentable)?;
    let mut output = BoundedOutput::new(capacity);
    let result = serde_json::to_writer(&mut output, message);
    match output.failure {
        Some(WriteFailure::Limit { observed }) => Err(IpcError::FrameTooLarge {
            declared: u64::try_from(observed)
                .map_err(|_error| IpcError::FrameLengthUnrepresentable)?,
            maximum: maximum.get(),
        }),
        Some(WriteFailure::Allocation { requested }) => {
            Err(IpcError::AllocationFailed { requested })
        }
        None => {
            result.map_err(|source| IpcError::MalformedMessage { source })?;
            Ok(output.bytes)
        }
    }
}
