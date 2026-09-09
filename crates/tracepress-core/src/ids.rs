use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;
use uuid::{ContextV7, Timestamp, Uuid, Variant};

const NANOS_PER_SECOND: u32 = 1_000_000_000;
const UUID_V7_MAX_TIMESTAMP_MILLISECONDS: u64 = (1_u64 << 48) - 1;
const UUID_V7_MAX_TIMESTAMP_SECONDS: u64 = UUID_V7_MAX_TIMESTAMP_MILLISECONDS / 1_000;
const UUID_V7_MAX_TIMESTAMP_SUBSEC_NANOS: u32 = 655_999_999;

/// A validated timestamp accepted by `UUIDv7` generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UuidV7Timestamp {
    unix_seconds: u64,
    subsec_nanos: u32,
}

impl UuidV7Timestamp {
    /// Validates timestamp components for deterministic `UUIDv7` generation.
    ///
    /// # Errors
    /// Returns [`UuidV7TimestampError`] when the components cannot be represented by `UUIDv7`.
    pub const fn new(unix_seconds: u64, subsec_nanos: u32) -> Result<Self, UuidV7TimestampError> {
        if subsec_nanos >= NANOS_PER_SECOND {
            Err(UuidV7TimestampError::InvalidSubsecondNanoseconds {
                value: subsec_nanos,
            })
        } else if unix_seconds > UUID_V7_MAX_TIMESTAMP_SECONDS
            || (unix_seconds == UUID_V7_MAX_TIMESTAMP_SECONDS
                && subsec_nanos > UUID_V7_MAX_TIMESTAMP_SUBSEC_NANOS)
        {
            Err(UuidV7TimestampError::ExceedsMillisecondCapacity {
                unix_seconds,
                subsec_nanos,
                maximum_milliseconds: UUID_V7_MAX_TIMESTAMP_MILLISECONDS,
            })
        } else {
            Ok(Self {
                unix_seconds,
                subsec_nanos,
            })
        }
    }
}

/// Failure to construct a valid `UUIDv7` timestamp.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum UuidV7TimestampError {
    /// The nanosecond component was outside the valid range.
    #[error("subsecond nanoseconds must be below 1,000,000,000, found {value}")]
    InvalidSubsecondNanoseconds {
        /// The rejected component.
        value: u32,
    },
    /// The timestamp exceeded the 48-bit millisecond field capacity.
    #[error(
        "timestamp {unix_seconds}s + {subsec_nanos}ns exceeds UUIDv7 maximum {maximum_milliseconds}ms"
    )]
    ExceedsMillisecondCapacity {
        /// The rejected whole-second component.
        unix_seconds: u64,
        /// The rejected subsecond nanosecond component.
        subsec_nanos: u32,
        /// The largest millisecond value representable by `UUIDv7`.
        maximum_milliseconds: u64,
    },
}

/// Stateful `UUIDv7` generator preserving order within equal timestamps.
pub struct UuidV7Generator(ContextV7);

impl UuidV7Generator {
    /// Creates an independent monotonic `UUIDv7` generation context.
    #[must_use]
    pub const fn new() -> Self {
        Self(ContextV7::new())
    }

    fn generate(&self) -> UuidV7 {
        UuidV7(Uuid::new_v7(Timestamp::now(&self.0)))
    }

    fn generate_at(&self, timestamp: UuidV7Timestamp) -> UuidV7 {
        UuidV7(Uuid::new_v7(Timestamp::from_unix(
            &self.0,
            timestamp.unix_seconds,
            timestamp.subsec_nanos,
        )))
    }
}

impl Default for UuidV7Generator {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for UuidV7Generator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UuidV7Generator")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
struct UuidV7(Uuid);

impl TryFrom<Uuid> for UuidV7 {
    type Error = IdParseError;

    fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
        if uuid.get_version_num() != 7 {
            Err(IdParseError::UnexpectedVersion {
                actual: uuid.get_version_num(),
            })
        } else if uuid.get_variant() != Variant::RFC4122 {
            Err(IdParseError::UnexpectedVariant {})
        } else {
            Ok(Self(uuid))
        }
    }
}

impl<'de> Deserialize<'de> for UuidV7 {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        Self::try_from(Uuid::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Failure to parse a typed external Tracepress ID.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum IdParseError {
    /// The input was not a syntactically valid UUID.
    #[error(transparent)]
    InvalidUuid(#[from] uuid::Error),
    /// The UUID had a version other than seven.
    #[error("expected UUIDv7, found UUIDv{actual}")]
    UnexpectedVersion {
        /// The rejected UUID version number.
        actual: usize,
    },
    /// The UUID did not use the RFC 4122 variant required by `UUIDv7`.
    #[error("expected RFC 4122 UUID variant")]
    UnexpectedVariant {},
}

macro_rules! define_uuid_v7_ids {
    ($($name:ident => $description:literal),+ $(,)?) => {
        $(
            #[doc = $description]
            #[derive(
                Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
            )]
            #[serde(transparent)]
            pub struct $name(UuidV7);

            impl $name {
                /// Generates an ID using the current time and the supplied monotonic context.
                #[must_use]
                pub fn generate(generator: &UuidV7Generator) -> Self {
                    Self(generator.generate())
                }

                /// Generates an ID at a validated timestamp for deterministic tests or imports.
                #[must_use]
                pub fn generate_at(
                    generator: &UuidV7Generator,
                    timestamp: UuidV7Timestamp,
                ) -> Self {
                    Self(generator.generate_at(timestamp))
                }

                /// Returns the validated `UUIDv7` value.
                #[must_use]
                pub const fn as_uuid(self) -> Uuid {
                    self.0.0
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    self.0.0.fmt(formatter)
                }
            }

            impl FromStr for $name {
                type Err = IdParseError;

                fn from_str(value: &str) -> Result<Self, Self::Err> {
                    Uuid::parse_str(value).map_err(IdParseError::from)?.try_into().map(Self)
                }
            }

            impl TryFrom<Uuid> for $name {
                type Error = IdParseError;

                fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
                    UuidV7::try_from(uuid).map(Self)
                }
            }
        )+
    };
}

define_uuid_v7_ids!(
    SessionId => "A Tracepress session identity.",
    OperationId => "An operation node identity in the causal graph.",
    RequestId => "A logical provider request identity.",
    AttemptId => "A provider request attempt identity.",
    ToolCallId => "An agent tool-call identity.",
    DecisionId => "A compression decision identity.",
    RecoveryId => "A recovery record identity.",
    EvaluationId => "An outcome evaluation identity.",
    PolicyAssignmentId => "A per-session policy assignment identity.",
    OccurrenceId => "An observed content occurrence identity.",
    BindingId => "A frozen content binding identity.",
    EventId => "An append-only domain event identity.",
);
