use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use tracepress_core::{MaxIpcQueueItems, QueueDecision, classify_ipc_queue};

use crate::IpcError;

/// Sending half of a core-sized bounded connection queue.
#[derive(Debug)]
pub struct ConnectionSender<Connection> {
    sender: mpsc::Sender<Connection>,
    maximum: MaxIpcQueueItems,
}

impl<Connection> Clone for ConnectionSender<Connection> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            maximum: self.maximum,
        }
    }
}

impl<Connection> ConnectionSender<Connection> {
    /// Attempts deterministic admission without waiting for capacity.
    ///
    /// # Errors
    /// Returns [`IpcError::QueueFull`] or [`IpcError::QueueClosed`] without unbounded buffering.
    pub fn try_send(&self, connection: Connection) -> Result<(), IpcError> {
        let used = self
            .sender
            .max_capacity()
            .checked_sub(self.sender.capacity())
            .ok_or(IpcError::QueueCapacityUnrepresentable)?;
        let current_items =
            u64::try_from(used).map_err(|_error| IpcError::QueueCapacityUnrepresentable)?;
        match classify_ipc_queue(current_items, 1, self.maximum) {
            QueueDecision::WithinLimit { total_items: _ } => {}
            QueueDecision::Reject {
                current_items: _,
                incoming_items: _,
                maximum_items,
            } => {
                return Err(IpcError::QueueFull {
                    maximum: maximum_items,
                });
            }
        }
        self.sender
            .try_send(connection)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_connection) => IpcError::QueueFull {
                    maximum: self.maximum.get(),
                },
                mpsc::error::TrySendError::Closed(_connection) => IpcError::QueueClosed,
            })
    }

    /// Waits for bounded capacity or explicit cancellation.
    ///
    /// # Errors
    /// Returns cancellation or queue closure; capacity is never expanded.
    pub async fn send(
        &self,
        connection: Connection,
        cancellation: &CancellationToken,
    ) -> Result<(), IpcError> {
        let permit = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(IpcError::Cancelled),
            result = self.sender.reserve() => result.map_err(|_error| IpcError::QueueClosed)?,
        };
        permit.send(connection);
        Ok(())
    }

    /// Returns currently available bounded slots.
    #[must_use]
    pub fn available_capacity(&self) -> usize {
        self.sender.capacity()
    }
}

/// Receiving half of a core-sized bounded connection queue.
#[derive(Debug)]
pub struct ConnectionReceiver<Connection> {
    receiver: mpsc::Receiver<Connection>,
}

impl<Connection> ConnectionReceiver<Connection> {
    /// Receives one admitted connection, or `None` after all senders close.
    pub async fn receive(&mut self) -> Option<Connection> {
        self.receiver.recv().await
    }
}

/// Creates a bounded channel from the validated core queue contract.
///
/// # Errors
/// Returns [`IpcError::QueueCapacityUnrepresentable`] if Tokio cannot represent the capacity.
pub fn bounded_connections<Connection>(
    maximum: MaxIpcQueueItems,
) -> Result<(ConnectionSender<Connection>, ConnectionReceiver<Connection>), IpcError> {
    let capacity = queue_capacity(maximum)?;
    let (sender, receiver) = mpsc::channel(capacity);
    Ok((
        ConnectionSender { sender, maximum },
        ConnectionReceiver { receiver },
    ))
}

fn queue_capacity(maximum: MaxIpcQueueItems) -> Result<usize, IpcError> {
    let requested = maximum.get();
    let supported = u64::try_from(Semaphore::MAX_PERMITS).map_or(u64::MAX, std::convert::identity);
    if requested > supported {
        return Err(IpcError::QueueCapacityUnrepresentable);
    }
    usize::try_from(requested).map_err(|_error| IpcError::QueueCapacityUnrepresentable)
}

#[cfg(test)]
mod tests {
    use tokio::sync::Semaphore;
    use tracepress_core::MaxIpcQueueItems;

    use super::queue_capacity;

    #[test]
    fn queue_capacity_accepts_tokio_exact_maximum_without_allocation()
    -> Result<(), Box<dyn std::error::Error>> {
        // Given
        let maximum = u64::try_from(Semaphore::MAX_PERMITS)?;
        let queue_items = MaxIpcQueueItems::new(maximum)?;

        // When
        let capacity = queue_capacity(queue_items)?;

        // Then
        assert_eq!(capacity, Semaphore::MAX_PERMITS);
        Ok(())
    }
}
