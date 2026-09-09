use serde::{Deserialize, Serialize};
use tracepress_core::{MaxRequestBodyBytes, MaxResponseBodyBytes, RequestId};

use crate::IpcError;

/// One transport-neutral request carrying opaque bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IpcRequest {
    request_id: RequestId,
    body: Vec<u8>,
}

impl IpcRequest {
    /// Creates a typed request after enforcing its opaque body limit.
    ///
    /// # Errors
    /// Returns [`IpcError::RequestBodyTooLarge`] when `body` exceeds `maximum`.
    pub fn new(
        request_id: RequestId,
        body: Vec<u8>,
        maximum: MaxRequestBodyBytes,
    ) -> Result<Self, IpcError> {
        let request = Self { request_id, body };
        request.validate_body(maximum)?;
        Ok(request)
    }

    /// Returns the logical request identity.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the exact opaque request body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub(crate) fn validate_body(&self, maximum: MaxRequestBodyBytes) -> Result<(), IpcError> {
        let observed = u64::try_from(self.body.len())
            .map_err(|_error| IpcError::FrameLengthUnrepresentable)?;
        if observed > maximum.get() {
            return Err(IpcError::RequestBodyTooLarge {
                observed,
                maximum: maximum.get(),
            });
        }
        Ok(())
    }
}

/// Explicit completion state and opaque response bytes.
#[allow(
    clippy::exhaustive_enums,
    reason = "IPC consumers must handle every non-success outcome explicitly"
)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseOutcome {
    /// A complete response body.
    Complete {
        /// Exact response bytes.
        body: Vec<u8>,
    },
    /// The operation ended without a complete response.
    Incomplete,
    /// The operation was cancelled before completion.
    Cancelled,
}

impl ResponseOutcome {
    /// Creates a complete outcome after enforcing its opaque body limit.
    ///
    /// # Errors
    /// Returns [`IpcError::ResponseBodyTooLarge`] when `body` exceeds `maximum`.
    pub fn complete(body: Vec<u8>, maximum: MaxResponseBodyBytes) -> Result<Self, IpcError> {
        let outcome = Self::Complete { body };
        outcome.validate_body(maximum)?;
        Ok(outcome)
    }

    pub(crate) fn validate_body(&self, maximum: MaxResponseBodyBytes) -> Result<(), IpcError> {
        match self {
            Self::Complete { body } => {
                let observed = u64::try_from(body.len())
                    .map_err(|_error| IpcError::FrameLengthUnrepresentable)?;
                if observed > maximum.get() {
                    return Err(IpcError::ResponseBodyTooLarge {
                        observed,
                        maximum: maximum.get(),
                    });
                }
                Ok(())
            }
            Self::Incomplete | Self::Cancelled => Ok(()),
        }
    }
}

/// One transport-neutral response. Authentication material is absent by construction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IpcResponse {
    request_id: RequestId,
    outcome: ResponseOutcome,
}

impl IpcResponse {
    /// Creates a response for a logical request.
    #[must_use]
    pub const fn new(request_id: RequestId, outcome: ResponseOutcome) -> Self {
        Self {
            request_id,
            outcome,
        }
    }

    /// Returns the logical request identity.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the explicit response outcome.
    #[must_use]
    pub const fn outcome(&self) -> &ResponseOutcome {
        &self.outcome
    }

    pub(crate) fn validate_body(&self, maximum: MaxResponseBodyBytes) -> Result<(), IpcError> {
        self.outcome.validate_body(maximum)
    }
}
