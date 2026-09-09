//! Bounded framing behavior through async byte-stream seams.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Serialize, Serializer, ser::SerializeSeq as _};
use tokio::io::{AsyncWriteExt as _, duplex};
use tokio_util::sync::CancellationToken;
use tracepress_core::MaxIpcFrameBytes;
use tracepress_ipc::{
    FrameConfig, FramePart, IpcError, IpcRequest, read_frame, read_message, write_message,
};

struct CountingSequence<'counter> {
    serialized: &'counter AtomicUsize,
    total: usize,
}

impl Serialize for CountingSequence<'_> {
    fn serialize<Output>(&self, serializer: Output) -> Result<Output::Ok, Output::Error>
    where
        Output: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.total))?;
        for value in 0..self.total {
            let _previous = self.serialized.fetch_add(1, Ordering::SeqCst);
            sequence.serialize_element(&value)?;
        }
        sequence.end()
    }
}

const fn frame_limit(value: u64) -> Result<MaxIpcFrameBytes, tracepress_core::LimitValueError> {
    MaxIpcFrameBytes::new(value)
}

#[tokio::test]
async fn typed_message_serialization_stops_at_the_frame_limit()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (mut writer, _reader) = duplex(64);
    let serialized = AtomicUsize::new(0);
    let message = CountingSequence {
        serialized: &serialized,
        total: 10_000,
    };
    let cancellation = CancellationToken::new();

    // When
    let result = write_message(
        &mut writer,
        &message,
        FrameConfig::new(frame_limit(16)?, &cancellation),
    )
    .await;

    // Then
    assert!(matches!(
        result,
        Err(IpcError::FrameTooLarge { maximum: 16, .. })
    ));
    assert!(serialized.load(Ordering::SeqCst) < message.total);
    Ok(())
}

#[tokio::test]
async fn frame_rejects_declaration_before_payload_when_oversized()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (mut writer, mut reader) = duplex(16);
    writer.write_all(&9_u32.to_be_bytes()).await?;
    writer.shutdown().await?;

    // When
    let cancellation = CancellationToken::new();
    let result = read_frame(
        &mut reader,
        FrameConfig::new(frame_limit(8)?, &cancellation),
    )
    .await;
    let Err(error) = result else {
        return Err("oversized declaration was accepted".into());
    };

    // Then
    assert!(matches!(
        error,
        IpcError::FrameTooLarge {
            declared: 9,
            maximum: 8
        }
    ));
    Ok(())
}

#[tokio::test]
async fn frame_reports_exact_header_progress_when_truncated()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (mut writer, mut reader) = duplex(16);
    writer.write_all(&[0, 1]).await?;
    writer.shutdown().await?;

    // When
    let cancellation = CancellationToken::new();
    let result = read_frame(
        &mut reader,
        FrameConfig::new(frame_limit(8)?, &cancellation),
    )
    .await;
    let Err(error) = result else {
        return Err("truncated header was accepted".into());
    };

    // Then
    assert!(matches!(
        error,
        IpcError::Truncated {
            part: FramePart::Header,
            expected: 4,
            received: 2
        }
    ));
    Ok(())
}

#[tokio::test]
async fn frame_reports_exact_payload_progress_when_truncated()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (mut writer, mut reader) = duplex(16);
    writer.write_all(&3_u32.to_be_bytes()).await?;
    writer.write_all(&[4, 5]).await?;
    writer.shutdown().await?;

    // When
    let cancellation = CancellationToken::new();
    let result = read_frame(
        &mut reader,
        FrameConfig::new(frame_limit(8)?, &cancellation),
    )
    .await;
    let Err(error) = result else {
        return Err("truncated payload was accepted".into());
    };

    // Then
    assert!(matches!(
        error,
        IpcError::Truncated {
            part: FramePart::Payload,
            expected: 3,
            received: 2
        }
    ));
    Ok(())
}

#[tokio::test]
async fn frame_read_returns_cancelled_when_waiting_for_header()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (_writer, mut reader) = duplex(16);
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    // When
    let result = read_frame(
        &mut reader,
        FrameConfig::new(frame_limit(8)?, &cancellation),
    )
    .await;
    let Err(error) = result else {
        return Err("cancelled read was accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::Cancelled));
    Ok(())
}

#[tokio::test]
async fn typed_frame_rejects_malformed_json_when_payload_is_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let (mut writer, mut reader) = duplex(32);
    writer.write_all(&4_u32.to_be_bytes()).await?;
    writer.write_all(b"nope").await?;

    // When
    let cancellation = CancellationToken::new();
    let result = read_message::<_, IpcRequest>(
        &mut reader,
        FrameConfig::new(frame_limit(32)?, &cancellation),
    )
    .await;
    let Err(error) = result else {
        return Err("malformed JSON was accepted".into());
    };

    // Then
    assert!(matches!(error, IpcError::MalformedMessage { source: _ }));
    Ok(())
}
