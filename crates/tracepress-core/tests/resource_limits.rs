//! Finite resource configuration contract tests.

use tracepress_core::{
    MaxCpuWorkUnits, MaxDecompressedBytes, MaxIpcFrameBytes, MaxIpcQueueItems, MaxJsonItems,
    MaxJsonNesting, MaxLineBytes, MaxProcessingTimeMs, MaxRawBytes, MaxRequestBodyBytes,
    MaxResponseBodyBytes, ResourceLimitField, ResourceLimits, ResourceLimitsConfig,
    ResourceLimitsError,
};

const RESOURCE_FIELDS: [ResourceLimitField; 11] = [
    ResourceLimitField::RawBytes,
    ResourceLimitField::RequestBodyBytes,
    ResourceLimitField::ResponseBodyBytes,
    ResourceLimitField::DecompressedBytes,
    ResourceLimitField::IpcFrameBytes,
    ResourceLimitField::IpcQueueItems,
    ResourceLimitField::JsonNesting,
    ResourceLimitField::JsonItems,
    ResourceLimitField::LineBytes,
    ResourceLimitField::ProcessingTimeMs,
    ResourceLimitField::CpuWorkUnits,
];

#[test]
fn every_resource_field_rejects_each_invalid_configuration_class() {
    // Given
    let above_u64 = i128::from(u64::MAX) + 1;

    for field in RESOURCE_FIELDS {
        let invalid_cases = [
            (None, ResourceLimitsError::Missing { field }),
            (Some(0), ResourceLimitsError::Zero { field }),
            (Some(-1), ResourceLimitsError::Invalid { field, value: -1 }),
            (
                Some(above_u64),
                ResourceLimitsError::Invalid {
                    field,
                    value: above_u64,
                },
            ),
        ];

        for (value, expected) in invalid_cases {
            let mut config = valid_config();
            set_config_field(&mut config, field, value);

            // When
            let result = ResourceLimits::try_from(config);

            // Then
            assert_eq!(result, Err(expected));
        }
    }
}

#[test]
fn resource_limits_validate_every_named_finite_boundary() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let config = valid_config();

    // When
    let limits = ResourceLimits::try_from(config)?;

    // Then
    assert_eq!(limits.max_raw_bytes, MaxRawBytes::new(1)?);
    assert_eq!(limits.max_request_body_bytes, MaxRequestBodyBytes::new(2)?);
    assert_eq!(
        limits.max_response_body_bytes,
        MaxResponseBodyBytes::new(3)?
    );
    assert_eq!(limits.max_decompressed_bytes, MaxDecompressedBytes::new(4)?);
    assert_eq!(limits.max_ipc_frame_bytes, MaxIpcFrameBytes::new(5)?);
    assert_eq!(limits.max_ipc_queue_items, MaxIpcQueueItems::new(6)?);
    assert_eq!(limits.max_json_nesting, MaxJsonNesting::new(7)?);
    assert_eq!(limits.max_json_items, MaxJsonItems::new(8)?);
    assert_eq!(limits.max_line_bytes, MaxLineBytes::new(9)?);
    assert_eq!(limits.max_processing_time_ms, MaxProcessingTimeMs::new(10)?);
    assert_eq!(limits.max_cpu_work_units, MaxCpuWorkUnits::new(11)?);
    Ok(())
}

const fn valid_config() -> ResourceLimitsConfig {
    ResourceLimitsConfig {
        max_raw_bytes: Some(1),
        max_request_body_bytes: Some(2),
        max_response_body_bytes: Some(3),
        max_decompressed_bytes: Some(4),
        max_ipc_frame_bytes: Some(5),
        max_ipc_queue_items: Some(6),
        max_json_nesting: Some(7),
        max_json_items: Some(8),
        max_line_bytes: Some(9),
        max_processing_time_ms: Some(10),
        max_cpu_work_units: Some(11),
    }
}

const fn set_config_field(
    config: &mut ResourceLimitsConfig,
    field: ResourceLimitField,
    value: Option<i128>,
) {
    match field {
        ResourceLimitField::RawBytes => config.max_raw_bytes = value,
        ResourceLimitField::RequestBodyBytes => config.max_request_body_bytes = value,
        ResourceLimitField::ResponseBodyBytes => config.max_response_body_bytes = value,
        ResourceLimitField::DecompressedBytes => config.max_decompressed_bytes = value,
        ResourceLimitField::IpcFrameBytes => config.max_ipc_frame_bytes = value,
        ResourceLimitField::IpcQueueItems => config.max_ipc_queue_items = value,
        ResourceLimitField::JsonNesting => config.max_json_nesting = value,
        ResourceLimitField::JsonItems => config.max_json_items = value,
        ResourceLimitField::LineBytes => config.max_line_bytes = value,
        ResourceLimitField::ProcessingTimeMs => config.max_processing_time_ms = value,
        ResourceLimitField::CpuWorkUnits => config.max_cpu_work_units = value,
    }
}
