//! Provider endpoint boundary for transparent Phase 1 forwarding.

mod domain;
mod json;
mod observer;
mod request;
mod response;
mod sse;

use std::sync::Arc;

use http::Uri;
use thiserror::Error;

/// Validated absolute endpoint for OpenAI-compatible provider routes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ProviderEndpoint {
    endpoint: Arc<str>,
}

impl ProviderEndpoint {
    /// Validates one absolute HTTP(S) provider endpoint.
    ///
    /// Both OpenAI-compatible Phase 1 chat completions and Phase 2 Responses routes are accepted.
    ///
    /// # Errors
    /// Returns a typed error for an invalid URI, unsupported scheme, missing authority, or wrong
    /// path.
    pub fn new(value: &str) -> Result<Self, ProviderEndpointError> {
        let uri = value
            .parse::<Uri>()
            .map_err(ProviderEndpointError::InvalidUri)?;
        match uri.scheme_str() {
            Some("http" | "https") => {}
            Some(_) | None => return Err(ProviderEndpointError::UnsupportedScheme),
        }
        if uri.authority().is_none() {
            return Err(ProviderEndpointError::MissingAuthority);
        }
        if !matches!(uri.path(), "/v1/chat/completions" | "/v1/responses") {
            return Err(ProviderEndpointError::WrongPath);
        }
        Ok(Self {
            endpoint: Arc::from(value),
        })
    }

    /// Returns the validated endpoint without allocating.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.endpoint
    }
}

/// Provider endpoint validation failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProviderEndpointError {
    /// URI syntax is invalid.
    #[error("invalid provider endpoint URI: {0}")]
    InvalidUri(http::uri::InvalidUri),
    /// Only HTTP and HTTPS upstreams are supported.
    #[error("provider endpoint must use http or https")]
    UnsupportedScheme,
    /// Absolute upstream authority is required.

    #[error("provider endpoint must include an authority")]
    MissingAuthority,
    /// Provider endpoint is not one of the supported OpenAI-compatible routes.
    #[error("provider endpoint path must be /v1/chat/completions or /v1/responses")]
    WrongPath,
}
pub use domain::{
    AnomalyFlags, ContentCaptureMode, LimitsError, MAX_RETAINED_USAGE_BYTES, NormalizedUsage,
    OPENAI_RESPONSES_PARSER_VERSION, ObservationError, ObservationInput, ObservationLimitValues,
    ObservationLimits, ObservationStatus, ProviderKind, ProviderProtocol, ProviderResponseState,
    RawProviderUsage, USAGE_NORMALIZER_VERSION, UsageError, UsageStatus,
};
pub use observer::{ObservationResult, OpenAiResponsesV1Observer, ProviderObserver};
pub use request::{OpenAiResponsesRequestObservation, RequestObservation, parse_request};
pub use response::{
    OpenAiResponsesResponseObservation, ResponseObservation, normalize_usage, parse_response,
};
pub use sse::{SseEvent, SseFramer, StreamingObserver};
