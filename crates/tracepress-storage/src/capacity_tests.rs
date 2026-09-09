use tempfile::TempDir;
use tokio::sync::Semaphore;
use tracepress_core::MaxIpcQueueItems;

use crate::test_support::TestResult;
use crate::writer::writer_queue_capacity;
use crate::{Durability, StorageConfig, StorageError, StorageWriter};

#[test]
fn writer_capacity_validation_covers_tokio_boundary_and_one_over() -> TestResult {
    // Given: Tokio's exact public maximum permit count and its immediate successor.
    let maximum = u64::try_from(Semaphore::MAX_PERMITS)?;
    let one_over = maximum
        .checked_add(1)
        .ok_or_else(|| std::io::Error::other("Tokio permit bound must leave room for one-over"))?;

    // When: both values cross the writer's allocation-free validation boundary.
    let accepted = writer_queue_capacity(MaxIpcQueueItems::new(maximum)?);
    let rejected = writer_queue_capacity(MaxIpcQueueItems::new(one_over)?);

    // Then: the exact maximum is accepted and one-over is rejected with stable context.
    assert!(matches!(accepted, Ok(capacity) if capacity == Semaphore::MAX_PERMITS));
    assert!(matches!(
        rejected,
        Err(StorageError::WriterQueueCapacityExceeded {
            requested,
            maximum: supported
        }) if requested == one_over && supported == maximum
    ));
    Ok(())
}

#[tokio::test]
async fn writer_rejects_capacity_above_tokio_limit_without_panicking() -> TestResult {
    // Given: a positive queue limit exactly one above Tokio's supported permit bound.
    let directory = TempDir::new()?;
    let one_over = Semaphore::MAX_PERMITS
        .checked_add(1)
        .ok_or_else(|| std::io::Error::other("Tokio permit bound must leave room for one-over"))?;
    let requested = u64::try_from(one_over)?;
    let maximum = u64::try_from(Semaphore::MAX_PERMITS)?;
    let queue_items = MaxIpcQueueItems::new(requested)?;
    let config = StorageConfig::new(
        directory.path().join("capacity.sqlite3"),
        Durability::Balanced,
        queue_items,
    );

    // When: the public writer startup future runs in an observed task.
    let outcome = tokio::spawn(StorageWriter::open(config)).await;

    // Then: startup returns an ordinary typed failure instead of unwinding the task.
    assert!(matches!(
        outcome,
        Ok(Err(StorageError::WriterQueueCapacityExceeded {
            requested: rejected,
            maximum: supported
        })) if rejected == requested && supported == maximum
    ));
    Ok(())
}
