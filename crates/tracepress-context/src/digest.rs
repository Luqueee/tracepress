//! Local-only `SHA-256` fingerprints over analyzed bytes.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const SHA256_BYTES: usize = 32;
const SHA256_HEX_CHARS: usize = SHA256_BYTES * 2;

/// A `SHA-256` digest over bytes the analyzer hashed without retaining them.
///
/// Deliberately distinct from [`tracepress_core::ContentId`]: a content identity names bytes
/// Tracepress stored as a content object, while a context digest names request bytes this phase
/// never persists. A context digest therefore never implies a stored object and is never
/// exported raw; a future export applies `HMAC(local_secret, digest)` instead.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContextDigest([u8; SHA256_BYTES]);

impl ContextDigest {
    /// Hashes exact bytes without decoding, normalizing, or retaining them.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Reconstructs a digest from the output of an incremental SHA-256 computation.
    #[must_use]
    pub(crate) const fn from_sha256_bytes(bytes: [u8; SHA256_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the fixed-size digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SHA256_BYTES] {
        &self.0
    }
}

impl fmt::Display for ContextDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ContextDigest {
    type Err = ContextDigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != SHA256_HEX_CHARS {
            return Err(ContextDigestParseError::InvalidLength {
                actual: value.len(),
            });
        }

        let mut digest = [0; SHA256_BYTES];
        for (output, pair) in digest.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            let [high_byte, low_byte] = pair else {
                return Err(ContextDigestParseError::InvalidLength {
                    actual: value.len(),
                });
            };
            let high = decode_nibble(*high_byte)?;
            let low = decode_nibble(*low_byte)?;
            *output = (high << 4) | low;
        }
        Ok(Self(digest))
    }
}

impl Serialize for ContextDigest {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContextDigest {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        deserializer.deserialize_str(ContextDigestVisitor)
    }
}

/// Accepts a hexadecimal digest without allocating an intermediate string.
struct ContextDigestVisitor;

impl de::Visitor<'_> for ContextDigestVisitor {
    type Value = ContextDigest;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a 64-character hexadecimal SHA-256 digest")
    }

    fn visit_str<ErrorType>(self, value: &str) -> Result<Self::Value, ErrorType>
    where
        ErrorType: de::Error,
    {
        value.parse().map_err(de::Error::custom)
    }
}

/// Failure to parse a hexadecimal context digest.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ContextDigestParseError {
    /// The digest was not exactly 64 hexadecimal characters.
    #[error("context digest must contain 64 hexadecimal characters, found {actual}")]
    InvalidLength {
        /// The rejected string length.
        actual: usize,
    },
    /// A character was not hexadecimal.
    #[error("context digest contains non-hex byte {byte:#04x}")]
    InvalidHex {
        /// The rejected ASCII byte.
        byte: u8,
    },
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "the match arms prove each subtraction is within one ASCII digit range"
)]
const fn decode_nibble(byte: u8) -> Result<u8, ContextDigestParseError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ContextDigestParseError::InvalidHex { byte }),
    }
}
