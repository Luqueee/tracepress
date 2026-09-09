//! `UUIDv7` timestamp-capacity contract tests.

use tracepress_core::{SessionId, UuidV7Generator, UuidV7Timestamp, UuidV7TimestampError};

const MAX_TIMESTAMP_MILLISECONDS: u64 = (1_u64 << 48) - 1;
const MAX_TIMESTAMP_SECONDS: u64 = 281_474_976_710;
const MAX_TIMESTAMP_SUBSEC_NANOS: u32 = 655_000_000;
const FIRST_OVERFLOW_SUBSEC_NANOS: u32 = 656_000_000;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn timestamp_accepts_last_uuidv7_millisecond() {
    // Given
    let unix_seconds = MAX_TIMESTAMP_SECONDS;
    let subsec_nanos = MAX_TIMESTAMP_SUBSEC_NANOS;

    // When
    let result = UuidV7Timestamp::new(unix_seconds, subsec_nanos);

    // Then
    assert!(result.is_ok());
}

#[test]
fn timestamp_rejects_first_millisecond_beyond_uuidv7_capacity() {
    // Given
    let unix_seconds = MAX_TIMESTAMP_SECONDS;
    let subsec_nanos = FIRST_OVERFLOW_SUBSEC_NANOS;

    // When
    let result = UuidV7Timestamp::new(unix_seconds, subsec_nanos);

    // Then
    assert_eq!(
        result,
        Err(UuidV7TimestampError::ExceedsMillisecondCapacity {
            unix_seconds,
            subsec_nanos,
            maximum_milliseconds: MAX_TIMESTAMP_MILLISECONDS,
        })
    );
}

#[test]
fn timestamp_rejects_maximum_seconds_without_arithmetic_overflow() {
    // Given
    let unix_seconds = u64::MAX;
    let subsec_nanos = 0;

    // When
    let result = UuidV7Timestamp::new(unix_seconds, subsec_nanos);

    // Then
    assert_eq!(
        result,
        Err(UuidV7TimestampError::ExceedsMillisecondCapacity {
            unix_seconds,
            subsec_nanos,
            maximum_milliseconds: MAX_TIMESTAMP_MILLISECONDS,
        })
    );
}

#[test]
fn invalid_subsecond_nanoseconds_remain_the_primary_error() {
    // Given
    let unix_seconds = u64::MAX;
    let subsec_nanos = 1_000_000_000;

    // When
    let result = UuidV7Timestamp::new(unix_seconds, subsec_nanos);

    // Then
    assert_eq!(
        result,
        Err(UuidV7TimestampError::InvalidSubsecondNanoseconds {
            value: subsec_nanos,
        })
    );
}

#[test]
fn maximum_timestamp_id_orders_after_epoch_id() -> TestResult {
    // Given
    let epoch = UuidV7Timestamp::new(0, 0)?;
    let maximum = UuidV7Timestamp::new(MAX_TIMESTAMP_SECONDS, MAX_TIMESTAMP_SUBSEC_NANOS)?;

    // When
    let epoch_id = SessionId::generate_at(&UuidV7Generator::new(), epoch);
    let maximum_id = SessionId::generate_at(&UuidV7Generator::new(), maximum);

    // Then
    assert!(epoch_id < maximum_id);
    Ok(())
}
