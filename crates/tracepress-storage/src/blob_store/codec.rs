#![allow(
    clippy::redundant_pub_crate,
    reason = "sibling blob-store modules share these implementation-only helpers"
)]
#![allow(
    clippy::too_many_arguments,
    reason = "bounded decoding requires reader, identity, declared length, and hard limit together"
)]

use std::io::{Read, Write};

use sha2::{Digest as _, Sha256};
use tracepress_core::{ContentId, RawContent};

use super::{BlobByteLimit, BlobError};

const IO_BUFFER_BYTES: usize = 16 * 1_024;
const ZSTD_WINDOW_LOG_MAX: u32 = 23;

pub(crate) fn copy_bounded(raw_bytes: &[u8], limit: BlobByteLimit) -> Result<Box<[u8]>, BlobError> {
    if raw_bytes.len() > limit.get() {
        return Err(BlobError::TooLarge {
            content_id: ContentId::from_bytes(raw_bytes),
            actual: byte_length(raw_bytes.len())?,
            maximum: limit.get_u64(),
        });
    }
    let mut copy = Vec::new();
    copy.try_reserve_exact(raw_bytes.len())
        .map_err(|_error| BlobError::Allocation {
            requested: raw_bytes.len(),
        })?;
    copy.extend_from_slice(raw_bytes);
    Ok(copy.into_boxed_slice())
}

pub(crate) fn validate_raw(
    content_id: ContentId,
    raw_bytes: &[u8],
) -> Result<RawContent, BlobError> {
    if ContentId::from_bytes(raw_bytes) != content_id {
        return Err(BlobError::Corrupt {
            content_id,
            detail: "bytes do not match their SHA-256 identity",
        });
    }
    Ok(RawContent::new(raw_bytes))
}

pub(crate) fn content_id(hasher: Sha256) -> Result<ContentId, BlobError> {
    format!("{:x}", hasher.finalize())
        .parse()
        .map_err(|_error| BlobError::DigestEncoding)
}

pub(crate) fn read_raw(
    mut reader: impl Read,
    content_id: ContentId,
    expected: Option<u64>,
    limit: BlobByteLimit,
) -> Result<RawContent, BlobError> {
    let bytes = read_bounded(&mut reader, content_id, expected, limit)?;
    validate_raw(content_id, &bytes)
}

pub(crate) fn read_zstd(
    reader: impl Read,
    content_id: ContentId,
    expected: Option<u64>,
    limit: BlobByteLimit,
) -> Result<RawContent, BlobError> {
    let mut decoder = zstd::stream::read::Decoder::new(reader).map_err(|source| {
        BlobError::io(
            "initialize zstd decoder",
            std::path::Path::new("<cas>"),
            source,
        )
    })?;
    decoder
        .window_log_max(ZSTD_WINDOW_LOG_MAX)
        .map_err(|source| {
            BlobError::io("bound zstd decoder", std::path::Path::new("<cas>"), source)
        })?;
    let bytes = read_bounded(&mut decoder, content_id, expected, limit)?;
    validate_raw(content_id, &bytes)
}

fn read_bounded(
    reader: &mut impl Read,
    content_id: ContentId,
    expected: Option<u64>,
    limit: BlobByteLimit,
) -> Result<Vec<u8>, BlobError> {
    if expected.is_some_and(|value| value > limit.get_u64()) {
        return Err(BlobError::TooLarge {
            content_id,
            actual: expected_value(expected),
            maximum: limit.get_u64(),
        });
    }
    let capacity = expected
        .map(usize::try_from)
        .transpose()
        .map_err(|_error| BlobError::LimitUnrepresentable {
            field: "persisted_blob_length",
            value: expected_value(expected),
        })?
        .unwrap_or(0);
    let mut output = Vec::new();
    output
        .try_reserve_exact(capacity)
        .map_err(|_error| BlobError::Allocation {
            requested: capacity,
        })?;
    let mut buffer = [0_u8; IO_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer).map_err(|source| {
            BlobError::io("read CAS object", std::path::Path::new("<cas>"), source)
        })?;
        if read == 0 {
            break;
        }
        let next = output.len().checked_add(read).ok_or(BlobError::TooLarge {
            content_id,
            actual: u64::MAX,
            maximum: limit.get_u64(),
        })?;
        if next > limit.get() {
            return Err(BlobError::TooLarge {
                content_id,
                actual: byte_length(next)?,
                maximum: limit.get_u64(),
            });
        }
        output
            .try_reserve(read)
            .map_err(|_error| BlobError::Allocation { requested: next })?;
        output.extend_from_slice(buffer.get(..read).ok_or(BlobError::Corrupt {
            content_id,
            detail: "reader returned an invalid byte count",
        })?);
    }
    let actual = byte_length(output.len())?;
    if expected.is_some_and(|value| value != actual) {
        return Err(BlobError::Corrupt {
            content_id,
            detail: "decoded length disagrees with database metadata",
        });
    }
    Ok(output)
}

fn byte_length(value: usize) -> Result<u64, BlobError> {
    u64::try_from(value).map_err(|_error| BlobError::LimitUnrepresentable {
        field: "blob_byte_length",
        value: u64::MAX,
    })
}

const fn expected_value(expected: Option<u64>) -> u64 {
    match expected {
        Some(value) => value,
        None => u64::MAX,
    }
}

pub(crate) struct BoundedWriter<Writer> {
    inner: Writer,
    remaining: usize,
}

impl<Writer> BoundedWriter<Writer> {
    pub(crate) const fn new(inner: Writer, limit: BlobByteLimit) -> Self {
        Self {
            inner,
            remaining: limit.get(),
        }
    }

    pub(crate) fn into_inner(self) -> Writer {
        self.inner
    }
}

impl<Writer: Write> Write for BoundedWriter<Writer> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.len() > self.remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                "compressed blob exceeds configured persistence bound",
            ));
        }
        let written = self.inner.write(buffer)?;
        self.remaining = self.remaining.saturating_sub(written);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub(crate) const fn zstd_window_log_max() -> u32 {
    ZSTD_WINDOW_LOG_MAX
}
