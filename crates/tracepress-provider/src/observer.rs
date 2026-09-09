//! Versioned provider observer capability shared by online and offline callers.

use crate::{
    ObservationError, ObservationInput, ProviderKind, ProviderProtocol, RequestObservation,
    ResponseObservation, parse_request, parse_response,
};

/// Result returned by an observer capability.
pub type ObservationResult<T> = Result<T, ObservationError>;

/// Provider-specific semantic observer independent of sockets and storage.
pub trait ProviderObserver {
    /// Canonical provider identity.
    fn provider(&self) -> ProviderKind;

    /// Canonical protocol identity.
    fn protocol(&self) -> ProviderProtocol;

    /// Observes bounded request bytes without modifying them.
    ///
    /// # Errors
    ///
    /// Returns an observation error when the bounded input is malformed or exceeds a resource
    /// limit.
    fn observe_request(&self, input: ObservationInput<'_>)
    -> ObservationResult<RequestObservation>;

    /// Observes bounded non-stream response bytes without modifying them.
    ///
    /// # Errors
    ///
    /// Returns an observation error when the bounded input is malformed or exceeds a resource
    /// limit.
    fn observe_response(
        &self,
        input: ObservationInput<'_>,
    ) -> ObservationResult<ResponseObservation>;
}

/// `OpenAI` Responses API v1 observer reusable for offline fixtures and backfill.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct OpenAiResponsesV1Observer;

impl OpenAiResponsesV1Observer {
    /// Creates the stateless Responses v1 observer.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Observes one bounded SSE stream incrementally.
    #[must_use]
    pub fn observe_stream(
        &self,
        limits: crate::ObservationLimits,
        chunks: &[&[u8]],
    ) -> crate::ResponseObservation {
        let mut observer = crate::StreamingObserver::new(limits);
        for chunk in chunks {
            if observer.push(chunk).is_err() {
                break;
            }
        }
        observer.finish()
    }
}

impl ProviderObserver for OpenAiResponsesV1Observer {
    fn provider(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    fn protocol(&self) -> ProviderProtocol {
        ProviderProtocol::OpenAiResponsesV1
    }

    fn observe_request(
        &self,
        input: ObservationInput<'_>,
    ) -> ObservationResult<RequestObservation> {
        Ok(parse_request(input))
    }

    fn observe_response(
        &self,
        input: ObservationInput<'_>,
    ) -> ObservationResult<ResponseObservation> {
        Ok(parse_response(input))
    }
}
