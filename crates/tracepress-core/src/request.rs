use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;

use crate::RequestId;

/// The allowlisted Phase 1 provider route.
#[allow(
    clippy::exhaustive_enums,
    reason = "Phase 1 intentionally exposes one forwarding route"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestRoute {
    /// OpenAI-compatible `/v1/chat/completions`.
    ChatCompletions,
}

/// The allowlisted method for a provider request.
#[allow(
    clippy::exhaustive_enums,
    reason = "Phase 1 intentionally exposes only POST forwarding"
)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestMethod {
    /// HTTP POST.
    Post,
}

/// A validated three-digit HTTP response status.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct HttpStatusCode(u16);

impl HttpStatusCode {
    /// Validates an HTTP response status code.
    ///
    /// # Errors
    /// Returns [`HttpStatusCodeError`] outside the inclusive range 100 through 599.
    pub const fn new(value: u16) -> Result<Self, HttpStatusCodeError> {
        if value >= 100 && value <= 599 {
            Ok(Self(value))
        } else {
            Err(HttpStatusCodeError { value })
        }
    }

    /// Returns the validated numeric status.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for HttpStatusCode {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        Self::new(u16::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Failure to construct an HTTP status code.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("HTTP status code must be between 100 and 599, found {value}")]
pub struct HttpStatusCodeError {
    value: u16,
}

/// Nullable response facts observed for a logical request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResponseMetadata {
    status_code: Option<HttpStatusCode>,
    response_bytes: Option<u64>,
    latency_us: Option<u64>,
}

impl ResponseMetadata {
    /// Creates response metadata while retaining unknown values as `None`.
    #[must_use]
    pub const fn new(
        status_code: Option<HttpStatusCode>,
        response_bytes: Option<u64>,
        latency_us: Option<u64>,
    ) -> Self {
        Self {
            status_code,
            response_bytes,
            latency_us,
        }
    }
}

/// Allowlisted request metadata with no provider headers or inferred usage.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestMetadata {
    request_id: RequestId,
    route: RequestRoute,
    method: RequestMethod,
    request_bytes: u64,
    #[serde(default)]
    status_code: Option<HttpStatusCode>,
    #[serde(default)]
    response_bytes: Option<u64>,
    #[serde(default)]
    latency_us: Option<u64>,
}

impl RequestMetadata {
    /// Creates transport metadata for the sole Phase 1 forwarding seam.
    #[must_use]
    pub const fn chat_completions(request_id: RequestId, request_bytes: u64) -> Self {
        Self {
            request_id,
            route: RequestRoute::ChatCompletions,
            method: RequestMethod::Post,
            request_bytes,
            status_code: None,
            response_bytes: None,
            latency_us: None,
        }
    }

    /// Returns a new value carrying observed response metadata.
    #[must_use]
    pub const fn with_response(&self, response: ResponseMetadata) -> Self {
        Self {
            request_id: self.request_id,
            route: self.route,
            method: self.method,
            request_bytes: self.request_bytes,
            status_code: response.status_code,
            response_bytes: response.response_bytes,
            latency_us: response.latency_us,
        }
    }

    /// Returns the logical request identity.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the allowlisted provider route.
    #[must_use]
    pub const fn route(&self) -> RequestRoute {
        self.route
    }

    /// Returns the allowlisted request method.
    #[must_use]
    pub const fn method(&self) -> RequestMethod {
        self.method
    }

    /// Returns the exact request body byte count.
    #[must_use]
    pub const fn request_bytes(&self) -> u64 {
        self.request_bytes
    }

    /// Returns the response status when observed.
    #[must_use]
    pub const fn status_code(&self) -> Option<HttpStatusCode> {
        self.status_code
    }

    /// Returns the response byte count when observed.
    #[must_use]
    pub const fn response_bytes(&self) -> Option<u64> {
        self.response_bytes
    }

    /// Returns the request latency when observed.
    #[must_use]
    pub const fn latency_us(&self) -> Option<u64> {
        self.latency_us
    }
}
