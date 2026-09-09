use std::num::NonZeroU64;

use serde::Deserialize;
use thiserror::Error;

macro_rules! define_limit_types {
    ($($name:ident => $description:literal),+ $(,)?) => {
        $(
            #[doc = $description]
            #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
            pub struct $name(NonZeroU64);

            impl $name {
                /// Creates a finite, positive resource limit.
                ///
                /// # Errors
                /// Returns [`LimitValueError`] when `value` is zero.
                pub const fn new(value: u64) -> Result<Self, LimitValueError> {
                    match NonZeroU64::new(value) {
                        Some(value) => Ok(Self(value)),
                        None => Err(LimitValueError),
                    }
                }

                /// Returns the positive configured value.
                #[must_use]
                pub const fn get(self) -> u64 {
                    self.0.get()
                }

                const fn from_non_zero(value: NonZeroU64) -> Self {
                    Self(value)
                }
            }
        )+
    };
}

define_limit_types!(
    MaxRawBytes => "Maximum bytes inspected or captured from one opaque raw input.",
    MaxRequestBodyBytes => "Maximum bytes accepted in one provider request body.",
    MaxResponseBodyBytes => "Maximum bytes accepted across one provider response stream.",
    MaxDecompressedBytes => "Maximum declared bytes permitted for future decompressed content.",
    MaxIpcFrameBytes => "Maximum payload bytes permitted in one IPC frame.",
    MaxIpcQueueItems => "Maximum items permitted in one IPC queue.",
    MaxJsonNesting => "Maximum structural JSON nesting metadata permitted for inspection.",
    MaxJsonItems => "Maximum structural JSON item metadata permitted for inspection.",
    MaxLineBytes => "Maximum bytes retained from one opaque line.",
    MaxProcessingTimeMs => "Maximum deterministic processing-time units in milliseconds.",
    MaxCpuWorkUnits => "Maximum deterministic CPU or work units for one operation.",
);

/// Failure to construct an individual finite resource limit.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("resource limit must be greater than zero")]
#[non_exhaustive]
pub struct LimitValueError;

/// Names every independently configured resource boundary.
#[allow(
    clippy::exhaustive_enums,
    reason = "configuration errors must identify every required resource limit"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceLimitField {
    /// Opaque raw input bytes.
    RawBytes,
    /// Provider request body bytes.
    RequestBodyBytes,
    /// Provider response body bytes.
    ResponseBodyBytes,
    /// Future decompressed content bytes.
    DecompressedBytes,
    /// IPC frame bytes.
    IpcFrameBytes,
    /// IPC queue item count.
    IpcQueueItems,
    /// JSON nesting metadata.
    JsonNesting,
    /// JSON item metadata.
    JsonItems,
    /// Opaque line bytes.
    LineBytes,
    /// Processing time in milliseconds.
    ProcessingTimeMs,
    /// CPU or abstract work units.
    CpuWorkUnits,
}

/// Unvalidated startup configuration in a representation that preserves invalid values.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::exhaustive_structs,
    reason = "startup config must expose the complete fixed set of required limits"
)]
pub struct ResourceLimitsConfig {
    /// Configured raw byte limit.
    pub max_raw_bytes: Option<i128>,
    /// Configured request body byte limit.
    pub max_request_body_bytes: Option<i128>,
    /// Configured response body byte limit.
    pub max_response_body_bytes: Option<i128>,
    /// Configured future decompressed byte limit.
    pub max_decompressed_bytes: Option<i128>,
    /// Configured IPC frame byte limit.
    pub max_ipc_frame_bytes: Option<i128>,
    /// Configured IPC queue item limit.
    pub max_ipc_queue_items: Option<i128>,
    /// Configured JSON nesting limit.
    pub max_json_nesting: Option<i128>,
    /// Configured JSON item limit.
    pub max_json_items: Option<i128>,
    /// Configured line byte limit.
    pub max_line_bytes: Option<i128>,
    /// Configured processing-time limit in milliseconds.
    pub max_processing_time_ms: Option<i128>,
    /// Configured CPU or abstract work-unit limit.
    pub max_cpu_work_units: Option<i128>,
}

/// Fully validated finite resource configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::exhaustive_structs,
    reason = "validated config must expose the complete fixed set of required limits"
)]
pub struct ResourceLimits {
    /// Maximum opaque raw input bytes.
    pub max_raw_bytes: MaxRawBytes,
    /// Maximum provider request body bytes.
    pub max_request_body_bytes: MaxRequestBodyBytes,
    /// Maximum provider response body bytes.
    pub max_response_body_bytes: MaxResponseBodyBytes,
    /// Maximum future decompressed bytes.
    pub max_decompressed_bytes: MaxDecompressedBytes,
    /// Maximum IPC frame bytes.
    pub max_ipc_frame_bytes: MaxIpcFrameBytes,
    /// Maximum IPC queue items.
    pub max_ipc_queue_items: MaxIpcQueueItems,
    /// Maximum JSON nesting metadata.
    pub max_json_nesting: MaxJsonNesting,
    /// Maximum JSON item metadata.
    pub max_json_items: MaxJsonItems,
    /// Maximum opaque line bytes.
    pub max_line_bytes: MaxLineBytes,
    /// Maximum processing time in milliseconds.
    pub max_processing_time_ms: MaxProcessingTimeMs,
    /// Maximum CPU or abstract work units.
    pub max_cpu_work_units: MaxCpuWorkUnits,
}

impl TryFrom<ResourceLimitsConfig> for ResourceLimits {
    type Error = ResourceLimitsError;

    fn try_from(config: ResourceLimitsConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            max_raw_bytes: MaxRawBytes::from_non_zero(parse_limit(
                config.max_raw_bytes,
                ResourceLimitField::RawBytes,
            )?),
            max_request_body_bytes: MaxRequestBodyBytes::from_non_zero(parse_limit(
                config.max_request_body_bytes,
                ResourceLimitField::RequestBodyBytes,
            )?),
            max_response_body_bytes: MaxResponseBodyBytes::from_non_zero(parse_limit(
                config.max_response_body_bytes,
                ResourceLimitField::ResponseBodyBytes,
            )?),
            max_decompressed_bytes: MaxDecompressedBytes::from_non_zero(parse_limit(
                config.max_decompressed_bytes,
                ResourceLimitField::DecompressedBytes,
            )?),
            max_ipc_frame_bytes: MaxIpcFrameBytes::from_non_zero(parse_limit(
                config.max_ipc_frame_bytes,
                ResourceLimitField::IpcFrameBytes,
            )?),
            max_ipc_queue_items: MaxIpcQueueItems::from_non_zero(parse_limit(
                config.max_ipc_queue_items,
                ResourceLimitField::IpcQueueItems,
            )?),
            max_json_nesting: MaxJsonNesting::from_non_zero(parse_limit(
                config.max_json_nesting,
                ResourceLimitField::JsonNesting,
            )?),
            max_json_items: MaxJsonItems::from_non_zero(parse_limit(
                config.max_json_items,
                ResourceLimitField::JsonItems,
            )?),
            max_line_bytes: MaxLineBytes::from_non_zero(parse_limit(
                config.max_line_bytes,
                ResourceLimitField::LineBytes,
            )?),
            max_processing_time_ms: MaxProcessingTimeMs::from_non_zero(parse_limit(
                config.max_processing_time_ms,
                ResourceLimitField::ProcessingTimeMs,
            )?),
            max_cpu_work_units: MaxCpuWorkUnits::from_non_zero(parse_limit(
                config.max_cpu_work_units,
                ResourceLimitField::CpuWorkUnits,
            )?),
        })
    }
}

/// Failure to validate complete startup resource configuration.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ResourceLimitsError {
    /// One mandatory limit was absent.
    #[error("missing required resource limit {field:?}")]
    Missing {
        /// The absent configuration field.
        field: ResourceLimitField,
    },
    /// One mandatory limit was zero.
    #[error("resource limit {field:?} must be greater than zero")]
    Zero {
        /// The zero-valued configuration field.
        field: ResourceLimitField,
    },
    /// One mandatory limit was negative or exceeded the supported integer range.
    #[error("resource limit {field:?} has invalid value {value}")]
    Invalid {
        /// The invalid configuration field.
        field: ResourceLimitField,
        /// The rejected signed value.
        value: i128,
    },
}

fn parse_limit(
    value: Option<i128>,
    field: ResourceLimitField,
) -> Result<NonZeroU64, ResourceLimitsError> {
    let value = value.ok_or(ResourceLimitsError::Missing { field })?;
    if value == 0 {
        return Err(ResourceLimitsError::Zero { field });
    }
    let value =
        u64::try_from(value).map_err(|_error| ResourceLimitsError::Invalid { field, value })?;
    NonZeroU64::new(value).ok_or(ResourceLimitsError::Zero { field })
}
