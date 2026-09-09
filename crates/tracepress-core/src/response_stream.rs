use crate::{ByteBoundaryError, ByteResource, MaxResponseBodyBytes};

/// Why a response stream was terminated before completion.
#[allow(
    clippy::exhaustive_enums,
    reason = "response termination reasons are consumed exhaustively by protocol layers"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseLimitReason {
    /// The next chunk would exceed the configured response body limit.
    BodyLimit,
    /// The cumulative byte counter could not represent the next total.
    CounterOverflow,
}

/// A deterministic response-stream accounting decision.
#[allow(
    clippy::exhaustive_enums,
    reason = "stream consumers must distinguish continuation from incomplete termination"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamDecision {
    /// The chunk fits and the stream may continue.
    Continue {
        /// Total bytes accepted after this chunk.
        accepted_bytes: u64,
    },
    /// The chunk is rejected and the stream must terminate incomplete.
    TerminateIncomplete {
        /// Total bytes accepted before this chunk.
        accepted_bytes: u64,
        /// Bytes in the rejected chunk.
        rejected_chunk_bytes: u64,
        /// The deterministic termination reason.
        reason: ResponseLimitReason,
    },
}

/// Allocation-free cumulative accounting for one streaming response.
///
/// Mutable accounting state cannot be duplicated into divergent counters.
///
/// ```compile_fail
/// use tracepress_core::StreamingResponseBudget;
///
/// fn require_clone<T: Clone>() {}
/// require_clone::<StreamingResponseBudget>();
/// ```
///
/// ```compile_fail
/// use tracepress_core::StreamingResponseBudget;
///
/// fn require_copy<T: Copy>() {}
/// require_copy::<StreamingResponseBudget>();
/// ```
#[derive(Debug, Eq, PartialEq)]
pub struct StreamingResponseBudget {
    maximum: MaxResponseBodyBytes,
    accepted_bytes: u64,
    terminated: bool,
}

impl StreamingResponseBudget {
    /// Creates an empty streaming response budget.
    #[must_use]
    pub const fn new(maximum: MaxResponseBodyBytes) -> Self {
        Self {
            maximum,
            accepted_bytes: 0,
            terminated: false,
        }
    }

    /// Accounts for one borrowed chunk without retaining or decoding it.
    ///
    /// # Errors
    /// Returns [`ByteBoundaryError`] after termination or for an unrepresentable chunk length.
    pub fn observe_chunk(&mut self, chunk: &[u8]) -> Result<StreamDecision, ByteBoundaryError> {
        if self.terminated {
            return Err(ByteBoundaryError::StreamAlreadyTerminated);
        }
        let chunk_bytes = u64::try_from(chunk.len()).map_err(|_error| {
            ByteBoundaryError::LengthUnrepresentable {
                resource: ByteResource::ResponseBody,
            }
        })?;
        let Some(total) = self.accepted_bytes.checked_add(chunk_bytes) else {
            self.terminated = true;
            return Ok(StreamDecision::TerminateIncomplete {
                accepted_bytes: self.accepted_bytes,
                rejected_chunk_bytes: chunk_bytes,
                reason: ResponseLimitReason::CounterOverflow,
            });
        };
        if total > self.maximum.get() {
            self.terminated = true;
            Ok(StreamDecision::TerminateIncomplete {
                accepted_bytes: self.accepted_bytes,
                rejected_chunk_bytes: chunk_bytes,
                reason: ResponseLimitReason::BodyLimit,
            })
        } else {
            self.accepted_bytes = total;
            Ok(StreamDecision::Continue {
                accepted_bytes: self.accepted_bytes,
            })
        }
    }

    /// Returns bytes accepted without retaining any response chunks.
    #[must_use]
    pub const fn accepted_bytes(&self) -> u64 {
        self.accepted_bytes
    }
}
