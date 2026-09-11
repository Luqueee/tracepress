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
    profile: ProviderEndpointProfile,
}

/// Versioned endpoint profile selected by a validated provider endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProviderEndpointProfile {
    /// OpenAI-compatible public API routes.
    OpenAiPublicApiV1,
    /// Fixed `ChatGPT` Codex subscription route.
    ChatGptCodexSubscriptionV1,
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
        if !matches!(
            uri.path(),
            "/v1/chat/completions" | "/v1/responses" | "/v1/responses/compact"
        ) {
            return Err(ProviderEndpointError::WrongPath);
        }
        Ok(Self {
            endpoint: Arc::from(value),
            profile: ProviderEndpointProfile::OpenAiPublicApiV1,
        })
    }

    /// Returns the validated endpoint without allocating.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.endpoint
    }

    /// Constructs the fixed `ChatGPT` Codex subscription endpoint.
    #[must_use]
    pub fn chatgpt_codex_subscription() -> Self {
        Self {
            endpoint: Arc::from("https://chatgpt.com/backend-api/codex"),
            profile: ProviderEndpointProfile::ChatGptCodexSubscriptionV1,
        }
    }

    /// Returns the transport represented by this endpoint profile.
    #[must_use]
    pub const fn transport(&self) -> ProviderTransport {
        match self.profile {
            ProviderEndpointProfile::OpenAiPublicApiV1 => ProviderTransport::OpenAiPublicApi,
            ProviderEndpointProfile::ChatGptCodexSubscriptionV1 => {
                ProviderTransport::ChatGptCodexSubscription
            }
        }
    }

    /// Returns the provider identity owned by this endpoint profile.
    #[must_use]
    pub const fn provider(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    /// Returns the protocol identity owned by this endpoint profile.
    #[must_use]
    pub const fn protocol(&self) -> ProviderProtocol {
        ProviderProtocol::OpenAiResponsesV1
    }

    /// Returns the version of the endpoint profile.
    #[must_use]
    pub const fn profile_version(&self) -> u32 {
        1
    }

    /// Maps an allowlisted incoming route to this profile's upstream path.
    ///
    /// # Errors
    /// Returns an error when the incoming path is not accepted by this profile.
    pub fn upstream_path(
        &self,
        incoming_path: &str,
    ) -> Result<&'static str, ProviderEndpointError> {
        match self.profile {
            ProviderEndpointProfile::OpenAiPublicApiV1 => match incoming_path {
                "/v1/chat/completions" => Ok("/v1/chat/completions"),
                "/v1/responses" => Ok("/v1/responses"),
                "/v1/responses/compact" => Ok("/v1/responses/compact"),
                _ => Err(ProviderEndpointError::WrongPath),
            },
            ProviderEndpointProfile::ChatGptCodexSubscriptionV1 => match incoming_path {
                "/v1/responses" => Ok("/backend-api/codex/responses"),
                "/v1/responses/compact" => Ok("/backend-api/codex/responses/compact"),
                _ => Err(ProviderEndpointError::SubscriptionResponsesOnly),
            },
        }
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
    #[error("provider endpoint path must be an allowlisted OpenAI route")]
    WrongPath,
    /// The subscription profile only accepts the Responses v1 routes.
    #[error("ChatGPT Codex subscription transport only supports Responses routes")]
    SubscriptionResponsesOnly,
}
pub use domain::{
    AnalysisDecodeStatus, AnomalyFlags, CompactionProtocol, CompactionTrigger, ContentCaptureMode,
    ContentEncoding, LimitsError, MAX_RETAINED_USAGE_BYTES, NormalizedUsage,
    OPENAI_RESPONSES_PARSER_VERSION, ObservationError, ObservationInput, ObservationLimitValues,
    ObservationLimits, ObservationStatus, ProviderKind, ProviderProtocol, ProviderRequestKind,
    ProviderResponseState, ProviderTransport, RawProviderUsage, USAGE_NORMALIZER_VERSION,
    UsageError, UsageStatus,
};
pub use observer::{ObservationResult, OpenAiResponsesV1Observer, ProviderObserver};
pub use request::{OpenAiResponsesRequestObservation, RequestObservation, parse_request};
pub use response::{
    OpenAiResponsesResponseObservation, ResponseObservation, normalize_usage, parse_response,
};
pub use sse::{SseEvent, SseFramer, StreamingObserver};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_endpoint_is_fixed_and_maps_only_responses() {
        let endpoint = ProviderEndpoint::chatgpt_codex_subscription();
        assert_eq!(endpoint.as_str(), "https://chatgpt.com/backend-api/codex");
        assert_eq!(
            endpoint.transport(),
            ProviderTransport::ChatGptCodexSubscription
        );
        assert_eq!(endpoint.profile_version(), 1);
        assert!(matches!(
            endpoint.upstream_path("/v1/responses"),
            Ok("/backend-api/codex/responses")
        ));
        assert!(matches!(
            endpoint.upstream_path("/v1/chat/completions"),
            Err(ProviderEndpointError::SubscriptionResponsesOnly)
        ));
        assert!(matches!(
            endpoint.upstream_path("/v1/responses/compact"),
            Ok("/backend-api/codex/responses/compact")
        ));
    }

    #[test]
    fn arbitrary_backend_api_urls_are_not_subscription_profiles() {
        assert!(matches!(
            ProviderEndpoint::new("https://example.com/backend-api/codex"),
            Err(ProviderEndpointError::WrongPath)
        ));
    }
}
