//! Deterministic bounded-channel admission and backpressure behavior.

use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;
use tracepress_core::MaxIpcQueueItems;
use tracepress_ipc::{IpcError, bounded_connections};

const fn queue_limit(value: u64) -> Result<MaxIpcQueueItems, tracepress_core::LimitValueError> {
    MaxIpcQueueItems::new(value)
}

#[tokio::test]
async fn bounded_queue_rejects_capacity_above_tokio_limit_without_panicking()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let one_over = Semaphore::MAX_PERMITS
        .checked_add(1)
        .ok_or_else(|| std::io::Error::other("Tokio permit bound must leave room for one-over"))?;
    let maximum = queue_limit(u64::try_from(one_over)?)?;

    // When
    let outcome = tokio::spawn(async move { bounded_connections::<u8>(maximum) }).await;

    // Then
    assert!(matches!(
        outcome,
        Ok(Err(IpcError::QueueCapacityUnrepresentable))
    ));
    Ok(())
}

#[test]
fn bounded_queue_reports_closed_after_receiver_drop() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (sender, receiver) = bounded_connections(queue_limit(1)?)?;
    drop(receiver);

    // When
    let result = sender.try_send(9_u8);

    // Then
    assert!(matches!(result, Err(IpcError::QueueClosed)));
    Ok(())
}

#[tokio::test]
async fn bounded_queue_rejects_immediately_when_capacity_is_full()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (sender, mut receiver) = bounded_connections(queue_limit(1)?)?;
    sender.try_send(11_u8)?;

    // When
    let Err(error) = sender.try_send(22_u8) else {
        return Err("full queue accepted another item".into());
    };

    // Then
    assert!(matches!(error, IpcError::QueueFull { maximum: 1 }));
    assert_eq!(receiver.receive().await, Some(11));
    Ok(())
}

#[tokio::test]
async fn bounded_queue_sender_resumes_only_after_receiver_frees_capacity()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (sender, mut receiver) = bounded_connections(queue_limit(1)?)?;
    sender.try_send(31_u8)?;
    let entered = std::sync::Arc::new(Notify::new());
    let task_entered = std::sync::Arc::clone(&entered);
    let waiting_sender = sender.clone();
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(async move {
        task_entered.notify_one();
        waiting_sender.send(32_u8, &cancellation).await
    });
    entered.notified().await;

    // When
    let first = receiver.receive().await;
    let send_result = task.await?;
    let second = receiver.receive().await;

    // Then
    assert_eq!(first, Some(31));
    assert!(send_result.is_ok());
    assert_eq!(second, Some(32));
    Ok(())
}

#[tokio::test]
async fn bounded_queue_wait_returns_cancelled_without_capacity()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (sender, _receiver) = bounded_connections(queue_limit(1)?)?;
    sender.try_send(41_u8)?;
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    // When
    let Err(error) = sender.send(42_u8, &cancellation).await else {
        return Err("cancelled queue send completed".into());
    };

    // Then
    assert!(matches!(error, IpcError::Cancelled));
    Ok(())
}
