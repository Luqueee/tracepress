//! Protocol and credential behavior through the public IPC API.

use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes, RequestId, UuidV7Generator};
use tracepress_ipc::{Credential, IpcError, IpcRequest, IpcResponse, ResponseOutcome};

#[test]
fn request_constructor_rejects_body_above_its_typed_limit() -> Result<(), Box<dyn std::error::Error>>
{
    // Given
    let request_id = RequestId::generate(&UuidV7Generator::new());
    let maximum = MaxRequestBodyBytes::new(3)?;

    // When
    let result = IpcRequest::new(request_id, vec![1, 2, 3, 4], maximum);

    // Then
    assert!(matches!(
        result,
        Err(IpcError::RequestBodyTooLarge {
            observed: 4,
            maximum: 3
        })
    ));
    Ok(())
}

#[test]
fn response_constructor_rejects_body_above_its_typed_limit()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let maximum = MaxResponseBodyBytes::new(3)?;

    // When
    let result = ResponseOutcome::complete(vec![1, 2, 3, 4], maximum);

    // Then
    assert!(matches!(
        result,
        Err(IpcError::ResponseBodyTooLarge {
            observed: 4,
            maximum: 3
        })
    ));
    Ok(())
}

#[test]
fn protocol_round_trip_preserves_nul_when_request_is_typed()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let request_id = RequestId::generate(&UuidV7Generator::new());
    let request = IpcRequest::new(request_id, vec![0, 1, 0, 255], MaxRequestBodyBytes::new(4)?)?;

    // When
    let encoded = serde_json::to_vec(&request)?;
    let decoded: IpcRequest = serde_json::from_slice(&encoded)?;

    // Then
    assert_eq!(decoded, request);
    Ok(())
}

#[test]
fn response_serialization_cannot_contain_authentication_when_completed()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let request_id = RequestId::generate(&UuidV7Generator::new());
    let response = IpcResponse::new(
        request_id,
        ResponseOutcome::complete(vec![7, 0, 9], MaxResponseBodyBytes::new(3)?)?,
    );

    // When
    let encoded = serde_json::to_vec(&response)?;

    // Then
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;
    let Some(object) = value.as_object() else {
        return Err("response was not an object".into());
    };
    assert_eq!(object.len(), 2);
    assert!(!object.contains_key("credential"));
    assert!(!object.contains_key("authentication"));
    Ok(())
}

#[test]
fn credential_debug_and_equality_do_not_expose_bytes() {
    // Given
    let credential = Credential::new([0x41; 32]);
    let matching = Credential::new([0x41; 32]);
    let different = Credential::new([0x42; 32]);

    // When
    let rendered = format!("{credential:?}");

    // Then
    assert_eq!(rendered, "Credential([REDACTED])");
    assert_eq!(credential, matching);
    assert_ne!(credential, different);
    assert!(!rendered.contains("41"));
}
