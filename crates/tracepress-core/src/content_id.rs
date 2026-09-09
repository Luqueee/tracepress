use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const SHA256_BYTES: usize = 32;
const SHA256_HEX_CHARS: usize = SHA256_BYTES * 2;

/// A SHA-256 digest of exact raw content bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentId([u8; SHA256_BYTES]);

impl ContentId {
    /// Computes the content identity without decoding or normalizing the input.
    #[must_use]
    pub fn from_bytes(raw_bytes: &[u8]) -> Self {
        Self(Sha256::digest(raw_bytes).into())
    }

    /// Returns the fixed-size digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SHA256_BYTES] {
        &self.0
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ContentId {
    type Err = ContentIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != SHA256_HEX_CHARS {
            return Err(ContentIdParseError::InvalidLength {
                actual: value.len(),
            });
        }

        let mut digest = [0; SHA256_BYTES];
        for (output, pair) in digest.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            let [high_byte, low_byte] = pair else {
                return Err(ContentIdParseError::InvalidLength {
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

impl Serialize for ContentId {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ContentId {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

/// Failure to parse a hexadecimal SHA-256 content identity.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ContentIdParseError {
    /// The digest was not exactly 64 hexadecimal characters.
    #[error("content ID must contain 64 hexadecimal characters, found {actual}")]
    InvalidLength {
        /// The rejected string length.
        actual: usize,
    },
    /// A character was not hexadecimal.
    #[error("content ID contains non-hex byte {byte:#04x}")]
    InvalidHex {
        /// The rejected ASCII byte.
        byte: u8,
    },
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "the match arms prove each subtraction is within one ASCII digit range"
)]
const fn decode_nibble(byte: u8) -> Result<u8, ContentIdParseError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ContentIdParseError::InvalidHex { byte }),
    }
}
