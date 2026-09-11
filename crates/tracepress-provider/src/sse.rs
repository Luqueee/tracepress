//! Incremental, bounded Server-Sent Events framing and Responses lifecycle observation.

use std::fmt;
use std::time::Instant;

use serde_json::Value;

use crate::domain::{
    ObservationError, ObservationLimits, ObservationStatus, ProviderResponseState, UsageStatus,
    status_for_error,
};
use crate::json::{self, StringExtraction};
use crate::response::{
    ResponseObservation, UsageOutcome, contains_compaction_item, extract_usage, is_terminal_state,
    settle_usage_status,
};

/// One completed SSE event. Data contains only that event's joined `data` fields.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct SseEvent {
    /// Optional SSE event name.
    pub event: Option<String>,
    /// UTF-8-independent event payload bytes.
    pub data: Box<[u8]>,
}

impl fmt::Debug for SseEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SseEvent")
            .field("event", &self.event)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// Bounded incremental SSE framer.
#[non_exhaustive]
pub struct SseFramer {
    limits: ObservationLimits,
    line: Vec<u8>,
    event_name: Option<String>,
    data: Vec<u8>,
    event_bytes: usize,
    event_count: usize,
    pending_cr: bool,
    /// First framing bound this framer crossed, latched so nothing further is ever framed.
    failure: Option<ObservationError>,
}

impl fmt::Debug for SseFramer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SseFramer")
            .field("limits", &self.limits)
            .field("line_len", &self.line.len())
            .field("event_name", &self.event_name)
            .field("data_len", &self.data.len())
            .field("event_bytes", &self.event_bytes)
            .field("event_count", &self.event_count)
            .field("pending_cr", &self.pending_cr)
            .field("failure", &self.failure)
            .finish()
    }
}

impl SseFramer {
    /// Creates an empty framer with finite event and event-count bounds.
    #[must_use]
    pub const fn new(limits: ObservationLimits) -> Self {
        Self {
            limits,
            line: Vec::new(),
            event_name: None,
            data: Vec::new(),
            event_bytes: 0,
            event_count: 0,
            pending_cr: false,
            failure: None,
        }
    }

    /// Feeds an arbitrary byte fragment and returns newly completed events.
    ///
    /// Events completed before a framing bound is crossed are returned rather than discarded, so
    /// where the transport split the stream cannot decide which events an observer interprets. The
    /// crossed bound still rejects the offending event and latches: nothing further is framed, and
    /// the error is reported by the next call.
    ///
    /// # Errors
    ///
    /// Returns [`ObservationError::ResourceLimit`] once a configured framing bound is exceeded: on
    /// the crossing call when it completed no event first, and on every call after it.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, ObservationError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let mut events = Vec::new();
        let framing = self.frame(bytes, &mut events);
        self.deliver(events, framing)
    }

    /// Frames one fragment into completed events, stopping at the first bound it crosses.
    fn frame(&mut self, bytes: &[u8], events: &mut Vec<SseEvent>) -> Result<(), ObservationError> {
        for &byte in bytes {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    self.finish_line(events)?;
                    continue;
                }
                self.finish_line(events)?;
            }
            match byte {
                b'\r' => self.pending_cr = true,
                b'\n' => self.finish_line(events)?,
                _ => {
                    self.event_bytes = self
                        .event_bytes
                        .checked_add(1)
                        .ok_or(ObservationError::ResourceLimit)?;
                    if self.event_bytes > self.limits.max_sse_event_bytes {
                        return Err(ObservationError::ResourceLimit);
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(())
    }

    /// Flushes a final unterminated line/event at end of stream.
    ///
    /// # Errors
    ///
    /// Returns [`ObservationError::ResourceLimit`] once a configured framing bound is exceeded,
    /// under the same rule as [`Self::push`].
    pub fn finish(&mut self) -> Result<Vec<SseEvent>, ObservationError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let mut events = Vec::new();
        let framing = self.flush(&mut events);
        self.deliver(events, framing)
    }

    /// Frames the final unterminated line and the event it belongs to.
    fn flush(&mut self, events: &mut Vec<SseEvent>) -> Result<(), ObservationError> {
        if self.pending_cr {
            self.pending_cr = false;
            self.finish_line(events)?;
        } else if !self.line.is_empty() {
            self.finish_line(events)?;
        }
        if self.event_name.is_some() || !self.data.is_empty() {
            self.emit_event(events)?;
        }
        Ok(())
    }

    /// Latches a framing failure while still handing back the events completed before it.
    ///
    /// The error is never dropped: the next call reports it, and the observer owning this framer
    /// reads the latched `failure` when it settles, so a bound crossed mid-fragment degrades the
    /// observation exactly as it does when a later fragment crosses it.
    fn deliver(
        &mut self,
        events: Vec<SseEvent>,
        framing: Result<(), ObservationError>,
    ) -> Result<Vec<SseEvent>, ObservationError> {
        match framing {
            Ok(()) => Ok(events),
            Err(error) => {
                self.failure = Some(error);
                if events.is_empty() {
                    Err(error)
                } else {
                    Ok(events)
                }
            }
        }
    }

    /// Alias for [`Self::push`] suited to tap integrations.
    ///
    /// # Errors
    ///
    /// Returns the error from [`Self::push`].
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, ObservationError> {
        self.push(bytes)
    }

    /// Alias for [`Self::finish`] at end of an SSE stream.
    ///
    /// # Errors
    ///
    /// Returns the error from [`Self::finish`].
    pub fn end(&mut self) -> Result<Vec<SseEvent>, ObservationError> {
        self.finish()
    }

    /// Number of completed events emitted so far.
    #[must_use]
    pub const fn event_count(&self) -> usize {
        self.event_count
    }

    fn finish_line(&mut self, events: &mut Vec<SseEvent>) -> Result<(), ObservationError> {
        let line = std::mem::take(&mut self.line);
        if line.is_empty() {
            if self.event_name.is_some() || !self.data.is_empty() {
                self.emit_event(events)?;
            }
            return Ok(());
        }
        if line.first() == Some(&b':') {
            return Ok(());
        }
        let (field, value) = line.iter().position(|byte| *byte == b':').map_or_else(
            || (line.as_slice(), &[][..]),
            |index| {
                let value_start = index.saturating_add(1);
                let value = line.get(value_start..).unwrap_or_default();
                let value = value.strip_prefix(b" ").unwrap_or(value);
                (line.get(..index).unwrap_or_default(), value)
            },
        );
        match field {
            b"event" => {
                if let Ok(value) = std::str::from_utf8(value) {
                    if value.len() <= self.limits.max_string_bytes {
                        self.event_name = Some(value.to_owned());
                    }
                }
            }
            b"data" => {
                let separator = usize::from(!self.data.is_empty());
                let next = self
                    .data
                    .len()
                    .checked_add(separator)
                    .and_then(|length| length.checked_add(value.len()))
                    .ok_or(ObservationError::ResourceLimit)?;
                if next > self.limits.max_sse_event_bytes {
                    return Err(ObservationError::ResourceLimit);
                }
                if separator == 1 {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value);
            }
            _ => {}
        }
        Ok(())
    }

    fn emit_event(&mut self, events: &mut Vec<SseEvent>) -> Result<(), ObservationError> {
        let next = self
            .event_count
            .checked_add(1)
            .ok_or(ObservationError::ResourceLimit)?;
        if next > self.limits.max_sse_events {
            return Err(ObservationError::ResourceLimit);
        }
        self.event_count = next;
        let event = SseEvent {
            event: self.event_name.take(),
            data: std::mem::take(&mut self.data).into_boxed_slice(),
        };
        self.event_bytes = 0;
        events.push(event);
        Ok(())
    }
}

/// Incremental semantic observer for a Responses v1 SSE stream.
#[non_exhaustive]
pub struct StreamingObserver {
    framer: SseFramer,
    clock: Instant,
    status: ObservationStatus,
    response: ResponseObservation,
    /// Terminal lifecycle state the provider itself reported, when one was observed.
    evidence: Option<ProviderResponseState>,
    /// Component completeness of the usage object observed so far.
    usage_evidence: Option<UsageStatus>,
    /// Whether the owner's terminal decision was already applied to this observation.
    settled: bool,
    /// Whether provider terminal evidence already froze semantic interpretation.
    terminal: bool,
}

impl fmt::Debug for StreamingObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamingObserver")
            .field("framer", &self.framer)
            .field("status", &self.status)
            .field("response_state", &self.response.response_state)
            .field("chunk_count", &self.response.chunk_count)
            .field("byte_count", &self.response.byte_count)
            .field("evidence", &self.evidence)
            .field("usage_evidence", &self.usage_evidence)
            .field("settled", &self.settled)
            .field("terminal", &self.terminal)
            .field("ttfb_us", &self.response.ttfb_us)
            .field("ttft_us", &self.response.ttft_us)
            .field("duration_us", &self.response.duration_us)
            .finish_non_exhaustive()
    }
}

impl StreamingObserver {
    /// Creates an observer with bounded framing resources.
    ///
    /// The returned observer measures its own elapsed time from this moment, so every reported
    /// duration is a real local measurement rather than a placeholder.
    #[must_use]
    pub fn new(limits: ObservationLimits) -> Self {
        let mut response = ResponseObservation::empty(ObservationStatus::Complete);
        response.chunk_count = Some(0);
        response.byte_count = Some(0);
        Self {
            framer: SseFramer::new(limits),
            clock: Instant::now(),
            status: ObservationStatus::Complete,
            response,
            evidence: None,
            usage_evidence: None,
            settled: false,
            terminal: false,
        }
    }

    /// Feeds one upstream fragment; no input is retained after semantic processing.
    ///
    /// Transport accounting covers every fragment the owner delivers until it settles the stream,
    /// so `chunk_count` and `byte_count` describe the bytes actually observed rather than where
    /// the transport happened to split the terminal event. Semantic interpretation stops at the
    /// terminal event itself, whichever fragment carried it.
    ///
    /// # Errors
    ///
    /// Returns an error when the fragment exceeds a configured framing resource bound. A bound
    /// crossed after this same fragment already completed events is reported by a later call
    /// instead, so the events framed before it are interpreted wherever the boundary fell.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), ObservationError> {
        if self.settled {
            return Ok(());
        }
        self.response.chunk_count = checked_add(self.response.chunk_count, 1, &mut self.status);
        self.response.byte_count = checked_add(
            self.response.byte_count,
            bytes.len() as u64,
            &mut self.status,
        );
        if self.terminal {
            // Trailing bytes after the terminal event — a `data: [DONE]` frame, a keep-alive
            // comment, a stray newline — are counted but never framed or interpreted.
            return Ok(());
        }
        if self.response.ttfb_us.is_none() && !bytes.is_empty() {
            self.response.ttfb_us = self.elapsed_us();
        }
        let events = match self.framer.push(bytes) {
            Ok(events) => events,
            Err(error) => {
                self.degrade(status_for_error(error));
                return Err(error);
            }
        };
        self.observe_events(events);
        Ok(())
    }

    /// Alias for [`Self::push`] used by stream taps.
    ///
    /// # Errors
    ///
    /// Returns the error from [`Self::push`].
    pub fn observe_chunk(&mut self, bytes: &[u8]) -> Result<(), ObservationError> {
        self.push(bytes)
    }

    /// Signals a clean stream end. Missing final events remain incomplete.
    pub fn finish(&mut self) -> ResponseObservation {
        if !self.settled {
            if !self.terminal {
                if let Ok(events) = self.framer.finish() {
                    self.observe_events(events);
                }
                self.absorb_framing_failure();
                if self.evidence.is_none() {
                    self.response.response_state = ProviderResponseState::Incomplete;
                    self.response.usage_status = self.unconfirmed_usage_status();
                    self.degrade(ObservationStatus::Partial);
                }
            }
            self.settle();
        }
        self.summary()
    }

    /// Signals client cancellation without claiming provider completion.
    pub fn cancel(&mut self) -> ResponseObservation {
        self.settle_without_completion(
            ProviderResponseState::Cancelled,
            ObservationStatus::Cancelled,
        )
    }

    /// Signals upstream disconnect without claiming provider completion.
    pub fn disconnect(&mut self) -> ResponseObservation {
        self.settle_without_completion(
            ProviderResponseState::Disconnected,
            ObservationStatus::Partial,
        )
    }

    /// Returns a clean summary, equivalent to [`Self::finish`] for an ended stream.
    #[must_use]
    pub fn summary(&self) -> ResponseObservation {
        let mut response = self.response.clone();
        response.status = self.status;
        response
    }

    /// Applies a terminal decision taken by the stream owner rather than by the provider.
    ///
    /// Terminal evidence the provider already reported is authoritative: a client cancellation or
    /// an upstream disconnect after a `response.completed` event keeps that lifecycle state and
    /// its usage instead of discarding evidence the observation already holds.
    fn settle_without_completion(
        &mut self,
        state: ProviderResponseState,
        status: ObservationStatus,
    ) -> ResponseObservation {
        if !self.settled {
            if !self.terminal {
                self.absorb_framing_failure();
            }
            if self.evidence.is_none() {
                self.response.response_state = state;
                self.response.usage_status = self.unconfirmed_usage_status();
                self.degrade(status);
            }
            self.settle();
        }
        self.summary()
    }

    /// Degrades the observation with the framing bound the framer latched, if it crossed one.
    ///
    /// The framer hands back the events it completed before a bound and reports the bound itself
    /// to whichever call comes next, so this is where a bound crossed by the last fragment the
    /// owner delivered still reaches the observation. It is consulted only while the framer is
    /// still being fed: bytes trailing the provider's own terminal event are never framed, so a
    /// bound they crossed inside that event's fragment is no more evidence than it is when a later
    /// fragment carries them.
    const fn absorb_framing_failure(&mut self) {
        if let Some(error) = self.framer.failure {
            self.degrade(status_for_error(error));
        }
    }

    /// Freezes semantic interpretation at the terminal instant.
    ///
    /// Transport accounting deliberately survives this: the observer keeps counting fragments the
    /// owner still delivers, so identical byte streams report identical counters however the
    /// transport fragmented them.
    fn freeze(&mut self) {
        if self.response.duration_us.is_none() {
            self.response.duration_us = self.elapsed_us();
        }
        self.terminal = true;
    }

    /// Applies the owner's terminal decision: nothing further is observed or counted.
    fn settle(&mut self) {
        self.freeze();
        self.settled = true;
    }

    /// Interprets newly framed events, cutting off at the terminal event rather than at a
    /// fragment boundary.
    fn observe_events(&mut self, events: Vec<SseEvent>) {
        for event in events {
            if self.terminal {
                return;
            }
            self.observe_event(&event);
        }
    }

    /// Usage availability for a stream that never reached a terminal provider state.
    fn unconfirmed_usage_status(&self) -> UsageStatus {
        self.usage_evidence
            .map_or(UsageStatus::Unavailable, |status| {
                settle_usage_status(status, false)
            })
    }

    /// Elapsed local microseconds since observation started.
    fn elapsed_us(&self) -> Option<u64> {
        u64::try_from(self.clock.elapsed().as_micros()).ok()
    }

    /// Degrades the observation status, never claiming a better outcome than already observed.
    const fn degrade(&mut self, status: ObservationStatus) {
        if severity(status) > severity(self.status) {
            self.status = status;
        }
    }

    fn observe_event(&mut self, event: &SseEvent) {
        let name = event.event.as_deref().unwrap_or_default();
        let lifecycle = match name {
            "response.created" | "response.queued" => Some(ProviderResponseState::Queued),
            "response.in_progress" => Some(ProviderResponseState::InProgress),
            "response.completed" => Some(ProviderResponseState::Completed),
            "response.incomplete" => Some(ProviderResponseState::Incomplete),
            "response.failed" | "error" => Some(ProviderResponseState::Failed),
            "response.cancelled" | "response.canceled" => Some(ProviderResponseState::Cancelled),
            _ => None,
        };
        let semantic_output =
            name.contains("output") && (name.contains("delta") || name.contains("item"));
        if semantic_output && self.response.ttft_us.is_none() {
            self.response.ttft_us = self.elapsed_us();
        }
        let limits = self.framer.limits;
        let payload = match json::parse_bounded(&event.data, limits) {
            Ok(payload) => payload,
            Err(error) => {
                if lifecycle.is_some() {
                    self.degrade(match error {
                        ObservationError::ResourceLimit => ObservationStatus::ResourceLimit,
                        ObservationError::InvalidText | ObservationError::Malformed => {
                            ObservationStatus::Partial
                        }
                    });
                }
                return;
            }
        };
        let (object, object_bytes) = payload.get("response").map_or_else(
            || (&payload, event.data.as_ref()),
            |nested| {
                (
                    nested,
                    json::member_span(&event.data, b"response").unwrap_or_default(),
                )
            },
        );
        if contains_compaction_item(&payload) {
            self.response.compaction_output_seen = true;
        }
        if let Some(object) = object.as_object() {
            let mut extraction = StringExtraction::new(limits.max_string_bytes);
            if self.response.provider_response_id.is_none() {
                self.response.provider_response_id = extraction.extract(object, "id");
            }
            if self.response.model.is_none() {
                self.response.model = extraction.extract(object, "model");
            }
            if self.response.incomplete_reason.is_none() {
                if let Some(Value::Object(details)) = object.get("incomplete_details") {
                    self.response.incomplete_reason = extraction.extract(details, "reason");
                }
            }
            if self.response.error_code.is_none() {
                if let Some(Value::Object(error)) = object.get("error") {
                    self.response.error_code = extraction.extract(error, "code");
                }
            }
            if extraction.partial {
                self.degrade(ObservationStatus::Partial);
            }
            let usage = extract_usage(object, object_bytes, limits.max_usage_bytes);
            match usage.outcome {
                UsageOutcome::Retained => {
                    self.usage_evidence = Some(usage.status);
                    self.response.normalized_usage = usage.normalized;
                    self.response.raw_usage = usage.raw;
                }
                UsageOutcome::Unusable => self.degrade(ObservationStatus::Partial),
                UsageOutcome::Oversized => self.degrade(ObservationStatus::ResourceLimit),
                UsageOutcome::Absent => {}
            }
        }
        if let Some(state) = lifecycle {
            if is_terminal_state(state) {
                // The provider itself reported the end of its own work, whichever end it was:
                // completed, incomplete, failed or cancelled. That evidence is authoritative and
                // final, so no later event contributes a field, a usage object or a lifecycle
                // change, whether it shared this fragment or arrived in a later one.
                self.evidence = Some(state);
                self.response.response_state = state;
                self.freeze();
            } else {
                // Only a stream with no terminal evidence yet reaches this: the terminal event
                // froze interpretation, so no lifecycle walk-back is reachable here.
                self.response.response_state = state;
            }
        }
        self.response.usage_status = self
            .usage_evidence
            .map_or(UsageStatus::Unavailable, |status| {
                settle_usage_status(status, self.evidence.is_some())
            });
        if self.evidence == Some(ProviderResponseState::Completed)
            && self.response.usage_status == UsageStatus::Unavailable
        {
            self.degrade(ObservationStatus::Partial);
        }
    }
}

fn checked_add(current: Option<u64>, amount: u64, status: &mut ObservationStatus) -> Option<u64> {
    current.unwrap_or(0).checked_add(amount).map_or_else(
        || {
            *status = ObservationStatus::ResourceLimit;
            None
        },
        Some,
    )
}

/// Ranks observation outcomes so a later event never claims a better outcome than one observed.
const fn severity(status: ObservationStatus) -> u8 {
    match status {
        ObservationStatus::Complete => 0,
        ObservationStatus::Partial => 1,
        ObservationStatus::Unsupported => 2,
        ObservationStatus::Cancelled => 3,
        ObservationStatus::ObserverBackpressure => 4,
        ObservationStatus::Malformed => 5,
        ObservationStatus::ResourceLimit => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Eq, PartialEq)]
    struct SemanticProjection<'a> {
        provider: crate::ProviderKind,
        protocol: crate::ProviderProtocol,
        parser_version: u32,
        status: ObservationStatus,
        provider_response_id: Option<&'a str>,
        model: Option<&'a str>,
        response_state: ProviderResponseState,
        incomplete_reason: Option<&'a str>,
        error_code: Option<&'a str>,
        raw_usage: Option<&'a [u8]>,
        normalized_usage: Option<&'a crate::NormalizedUsage>,
        usage_status: UsageStatus,
        compaction_output_seen: bool,
    }

    fn semantic_projection(observation: &ResponseObservation) -> SemanticProjection<'_> {
        SemanticProjection {
            provider: observation.provider,
            protocol: observation.protocol,
            parser_version: observation.parser_version,
            status: observation.status,
            provider_response_id: observation.provider_response_id.as_deref(),
            model: observation.model.as_deref(),
            response_state: observation.response_state,
            incomplete_reason: observation.incomplete_reason.as_deref(),
            error_code: observation.error_code.as_deref(),
            raw_usage: observation.raw_usage.as_ref().map(AsRef::as_ref),
            normalized_usage: observation.normalized_usage.as_ref(),
            usage_status: observation.usage_status,
            compaction_output_seen: observation.compaction_output_seen,
        }
    }

    fn stream_with_chunks(bytes: &[u8], width: usize) -> ResponseObservation {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        for chunk in bytes.chunks(width) {
            assert!(observer.push(chunk).is_ok());
        }
        observer.finish()
    }

    const COMPLETED_STREAM: &[u8] = b"event: response.created\r\ndata: {\"response\":{\"id\":\"resp_1\",\"model\":\"gpt\"}}\r\n\r\nevent: response.output_text.delta\ndata: {\"delta\":\"secret\"}\n\nevent: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\n";

    const COMPACTION_STREAM: &[u8] = b"event: response.created\ndata: {\"response\":{\"id\":\"resp_compact\"}}\n\nevent: response.output_item.done\ndata: {\"item\":{\"type\":\"compaction\",\"encrypted_content\":\"PRIVATE\"}}\n\nevent: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":4,\"output_tokens\":1,\"total_tokens\":5}}}\n\n";

    #[test]
    fn fragmentation_is_semantically_invariant() {
        let stream = COMPLETED_STREAM;
        let one = stream_with_chunks(stream, 1);
        let two = stream_with_chunks(stream, 2);
        let seven = stream_with_chunks(stream, 7);
        assert_eq!(semantic_projection(&one), semantic_projection(&two));
        assert_eq!(semantic_projection(&one), semantic_projection(&seven));
        assert_eq!(one.byte_count, Some(stream.len() as u64));
        assert_eq!(two.byte_count, Some(stream.len() as u64));
        assert_eq!(seven.byte_count, Some(stream.len() as u64));
        assert_eq!(one.chunk_count, Some(stream.len() as u64));
        assert_eq!(two.chunk_count, Some(stream.len().div_ceil(2) as u64));
        assert_eq!(seven.chunk_count, Some(stream.len().div_ceil(7) as u64));
        assert_eq!(one.response_state, ProviderResponseState::Completed);
        assert_eq!(one.usage_status, UsageStatus::Final);
    }

    #[test]
    fn a_compaction_output_item_is_detected_across_sse_fragments() {
        let observation = stream_with_chunks(COMPACTION_STREAM, 3);

        assert!(observation.compaction_output_seen);
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
        assert_eq!(observation.usage_status, UsageStatus::Final);
        let debug = format!("{observation:?}");
        assert!(!debug.contains("PRIVATE"));
    }

    /// A gateway that appends one more non-lifecycle event after the terminal event.
    const TRAILING_EVENT_STREAM: &[u8] = b"event: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\nevent: response.output_item.done\ndata: {\"response\":{\"id\":\"resp_late\",\"model\":\"late\"}}\n\n";

    /// A gateway that appends a keep-alive comment, a `[DONE]` sentinel and a stray newline.
    const TRAILING_BYTES_STREAM: &[u8] = b"event: response.completed\ndata: {\"response\":{\"id\":\"resp_2\",\"status\":\"completed\",\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\n: keep-alive\n\ndata: [DONE]\n\n\n";

    /// Asserts one byte stream is observed identically whole and at every fixed fragment width,
    /// in semantics and in transport accounting, and returns the whole-body observation.
    fn assert_fragmentation_invariant(stream: &[u8]) -> ResponseObservation {
        let whole = stream_with_chunks(stream, stream.len().max(1));
        assert_eq!(whole.chunk_count, Some(1));
        assert_eq!(whole.byte_count, Some(stream.len() as u64));
        for width in [1_usize, 2, 7, 64] {
            let fragmented = stream_with_chunks(stream, width);
            assert_eq!(
                semantic_projection(&fragmented),
                semantic_projection(&whole),
                "fragment width {width} must not change the observation"
            );
            assert_eq!(
                fragmented.byte_count,
                Some(stream.len() as u64),
                "fragment width {width} must count every delivered byte"
            );
            assert_eq!(
                fragmented.chunk_count,
                Some(stream.len().div_ceil(width) as u64),
                "fragment width {width} must count every delivered fragment"
            );
        }
        whole
    }

    #[test]
    fn an_event_after_the_terminal_event_is_ignored_however_the_transport_split_it() {
        let observation = assert_fragmentation_invariant(TRAILING_EVENT_STREAM);
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
        assert_eq!(observation.usage_status, UsageStatus::Final);
        assert_eq!(
            observation.provider_response_id, None,
            "an event after the terminal one must never contribute metadata"
        );
        assert_eq!(observation.model, None);
    }

    #[test]
    fn trailing_bytes_after_the_terminal_event_are_counted_and_never_interpreted() {
        let observation = assert_fragmentation_invariant(TRAILING_BYTES_STREAM);
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
        assert_eq!(observation.usage_status, UsageStatus::Final);
        assert_eq!(observation.status, ObservationStatus::Complete);
        assert_eq!(observation.provider_response_id.as_deref(), Some("resp_2"));
    }

    /// Bounds a merely long or deliberately hostile stream actually reaches, matching the
    /// configuration the crate's own fuzz targets use.
    fn bounded_limits() -> ObservationLimits {
        ObservationLimits {
            max_sse_event_bytes: 8 * 1024,
            max_sse_events: 128,
            ..ObservationLimits::default()
        }
    }

    /// Drives one byte stream at a fixed fragment width and settles it, tolerating the framing
    /// bound the owner is told about: which call reports a crossed bound depends on where the
    /// transport split the stream, what was observed must not.
    fn bounded_stream(
        bytes: &[u8],
        width: usize,
        limits: ObservationLimits,
    ) -> ResponseObservation {
        let mut observer = StreamingObserver::new(limits);
        for chunk in bytes.chunks(width) {
            let _reported = observer.push(chunk);
        }
        observer.finish()
    }

    /// Asserts one byte stream yields one semantic projection delivered whole — the single
    /// fragment a gateway that buffers the response body hands over — and at every fixed width,
    /// counting every delivered byte and fragment either way. Returns the whole-body observation.
    fn assert_bounded_invariance(stream: &[u8], limits: ObservationLimits) -> ResponseObservation {
        let whole = bounded_stream(stream, stream.len().max(1), limits);
        assert_eq!(whole.chunk_count, Some(1));
        assert_eq!(whole.byte_count, Some(stream.len() as u64));
        for width in [1_usize, 7, 64] {
            let fragmented = bounded_stream(stream, width, limits);
            assert_eq!(
                semantic_projection(&fragmented),
                semantic_projection(&whole),
                "fragment width {width} must not change the observation"
            );
            assert_eq!(
                fragmented.byte_count,
                Some(stream.len() as u64),
                "fragment width {width} must count every delivered byte"
            );
            assert_eq!(
                fragmented.chunk_count,
                Some(stream.len().div_ceil(width) as u64),
                "fragment width {width} must count every delivered fragment"
            );
        }
        whole
    }

    /// A terminal event carrying the identity, model and complete usage a consumer reads.
    const COMPLETED_EVENT: &[u8] = b"event: response.completed\ndata: {\"response\":{\"id\":\"resp_1\",\"model\":\"gpt\",\"status\":\"completed\",\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\n";

    /// The same terminal event with a different identity, used where a bound must reject it.
    const OVER_BOUND_COMPLETED_EVENT: &[u8] = b"event: response.completed\ndata: {\"response\":{\"id\":\"resp_over_bound\",\"model\":\"over\",\"status\":\"completed\",\"usage\":{\"input_tokens\":7,\"output_tokens\":7,\"total_tokens\":14}}}\n\n";

    /// A non-terminal opening event, carrying the identity a stream reports before it ends.
    const CREATED_EVENT: &[u8] =
        b"event: response.created\ndata: {\"response\":{\"id\":\"resp_1\",\"model\":\"gpt\"}}\n\n";

    /// One tiny event, of the kind a long generation emits by the thousand.
    const TINY_EVENT: &[u8] = b"data: x\n\n";

    /// One `data` line above the per-event byte bound, as one enormous provider frame is.
    fn oversized_line(limits: ObservationLimits) -> Vec<u8> {
        let mut line = b"data: ".to_vec();
        line.resize(limits.max_sse_event_bytes.saturating_add(16), b'x');
        line.extend_from_slice(b"\n\n");
        line
    }

    /// Builds a stream from a lead event, `count` tiny events and a trailing frame.
    fn stream_of(lead: &[u8], count: usize, tail: &[u8]) -> Vec<u8> {
        let mut stream = lead.to_vec();
        for _ in 0..count {
            stream.extend_from_slice(TINY_EVENT);
        }
        stream.extend_from_slice(tail);
        stream
    }

    #[test]
    fn a_terminal_event_survives_a_whole_body_delivery_that_crosses_the_event_count_bound() {
        let limits = bounded_limits();
        let stream = stream_of(COMPLETED_EVENT, 200, b"");
        let observation = assert_bounded_invariance(&stream, limits);
        assert_eq!(observation.status, ObservationStatus::Complete);
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
        assert_eq!(
            observation.provider_response_id.as_deref(),
            Some("resp_1"),
            "a bound crossed after the terminal event must not discard it"
        );
        assert_eq!(observation.model.as_deref(), Some("gpt"));
        assert_eq!(observation.usage_status, UsageStatus::Final);
    }

    #[test]
    fn a_terminal_event_survives_a_whole_body_delivery_that_crosses_the_event_byte_bound() {
        let limits = bounded_limits();
        let stream = stream_of(COMPLETED_EVENT, 0, &oversized_line(limits));
        let observation = assert_bounded_invariance(&stream, limits);
        assert_eq!(observation.status, ObservationStatus::Complete);
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
        assert_eq!(observation.provider_response_id.as_deref(), Some("resp_1"));
        assert_eq!(observation.usage_status, UsageStatus::Final);
    }

    #[test]
    fn metadata_framed_before_the_event_count_bound_survives_a_whole_body_delivery() {
        let limits = bounded_limits();
        let stream = stream_of(CREATED_EVENT, 200, b"");
        let observation = assert_bounded_invariance(&stream, limits);
        assert_eq!(
            observation.provider_response_id.as_deref(),
            Some("resp_1"),
            "a stream with no terminal event still reported its identity before the bound"
        );
        assert_eq!(observation.model.as_deref(), Some("gpt"));
        assert_eq!(
            observation.status,
            ObservationStatus::ResourceLimit,
            "the crossed bound is still reported, whichever call reports it"
        );
        assert_eq!(
            observation.response_state,
            ProviderResponseState::Incomplete
        );
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
    }

    #[test]
    fn metadata_framed_before_the_event_byte_bound_survives_a_whole_body_delivery() {
        let limits = bounded_limits();
        let stream = stream_of(CREATED_EVENT, 0, &oversized_line(limits));
        let observation = assert_bounded_invariance(&stream, limits);
        assert_eq!(observation.provider_response_id.as_deref(), Some("resp_1"));
        assert_eq!(observation.model.as_deref(), Some("gpt"));
        assert_eq!(observation.status, ObservationStatus::ResourceLimit);
        assert_eq!(
            observation.response_state,
            ProviderResponseState::Incomplete
        );
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
    }

    #[test]
    fn an_over_bound_event_is_never_interpreted_however_the_transport_split_it() {
        let limits = bounded_limits();
        let stream = stream_of(CREATED_EVENT, 200, OVER_BOUND_COMPLETED_EVENT);
        let observation = assert_bounded_invariance(&stream, limits);
        assert_eq!(
            observation.response_state,
            ProviderResponseState::Incomplete,
            "an event beyond the bound never moves the lifecycle"
        );
        assert_eq!(
            observation.provider_response_id.as_deref(),
            Some("resp_1"),
            "the identity of the rejected event is never interpreted"
        );
        assert_eq!(observation.model.as_deref(), Some("gpt"));
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
        assert_eq!(observation.status, ObservationStatus::ResourceLimit);
    }

    #[test]
    fn the_framer_hands_back_events_completed_before_the_count_bound_and_stops_framing() {
        let limits = bounded_limits();
        let stream = stream_of(CREATED_EVENT, 200, OVER_BOUND_COMPLETED_EVENT);
        let mut framer = SseFramer::new(limits);
        let events = framer.push(&stream).unwrap_or_default();
        assert_eq!(
            events.len(),
            limits.max_sse_events,
            "every event completed before the bound is handed back"
        );
        assert_eq!(
            framer.event_count(),
            limits.max_sse_events,
            "the bound still stops framing at the configured event count"
        );
        assert!(
            events
                .iter()
                .all(|event| event.event.as_deref() != Some("response.completed")),
            "the event the bound rejected must never be framed"
        );
        assert!(
            framer.push(TINY_EVENT).is_err(),
            "the crossed bound is reported by the next call and latches"
        );
        assert!(framer.finish().is_err());
        assert_eq!(framer.event_count(), limits.max_sse_events);
    }

    #[test]
    fn the_framer_rejects_an_over_bound_line_and_keeps_the_events_completed_before_it() {
        let limits = bounded_limits();
        let stream = stream_of(CREATED_EVENT, 0, &oversized_line(limits));
        let mut framer = SseFramer::new(limits);
        let events = framer.push(&stream).unwrap_or_default();
        assert_eq!(events.len(), 1);
        assert!(
            events
                .iter()
                .all(|event| event.data.len() <= limits.max_sse_event_bytes),
            "no framed event may carry more data than the per-event bound"
        );
        assert_eq!(
            framer.event_count(),
            1,
            "the over-bound line never becomes an event"
        );
        assert!(framer.push(TINY_EVENT).is_err());
        assert!(framer.finish().is_err());
    }

    /// A later well-formed event that would overwrite every field a terminal event established.
    const LATER_EVENT: &[u8] = b"event: response.in_progress\ndata: {\"response\":{\"id\":\"resp_late\",\"model\":\"late\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"error\":{\"code\":\"late_error\"},\"usage\":{\"input_tokens\":9,\"output_tokens\":9,\"total_tokens\":18}}}\n\n";

    /// A terminal lifecycle event carrying identity, model and a complete usage object.
    fn terminal_event(name: &str, status: &str) -> Vec<u8> {
        format!(
            "event: {name}\ndata: {{\"response\":{{\"id\":\"resp_1\",\"model\":\"gpt\",\"status\":\"{status}\",\"usage\":{{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}}}}\n\n"
        )
        .into_bytes()
    }

    #[test]
    fn no_terminal_lifecycle_event_is_walked_back_by_a_later_event() {
        for (name, status, state) in [
            (
                "response.completed",
                "completed",
                ProviderResponseState::Completed,
            ),
            ("response.failed", "failed", ProviderResponseState::Failed),
            (
                "response.incomplete",
                "incomplete",
                ProviderResponseState::Incomplete,
            ),
            (
                "response.cancelled",
                "cancelled",
                ProviderResponseState::Cancelled,
            ),
        ] {
            let mut stream = terminal_event(name, status);
            stream.extend_from_slice(LATER_EVENT);
            let observation = assert_fragmentation_invariant(&stream);
            assert_eq!(
                observation.response_state, state,
                "{name} is terminal provider evidence"
            );
            assert_eq!(
                observation.provider_response_id.as_deref(),
                Some("resp_1"),
                "{name} must keep the identity it reported"
            );
            assert_eq!(observation.model.as_deref(), Some("gpt"));
            assert_eq!(
                observation.incomplete_reason, None,
                "no field is interpreted after {name}"
            );
            assert_eq!(observation.error_code, None);
            assert_eq!(
                observation
                    .normalized_usage
                    .as_ref()
                    .and_then(|usage| usage.total),
                Some(3),
                "a later usage object must not replace the usage {name} reported"
            );
            assert_eq!(observation.usage_status, UsageStatus::Final);
            assert_eq!(observation.status, ObservationStatus::Complete);
        }
    }

    #[test]
    fn a_settled_observer_observes_and_counts_nothing_further() {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(observer.push(COMPLETED_STREAM).is_ok());
        let settled = observer.finish();
        assert!(observer.push(b"data: [DONE]\n\n").is_ok());
        assert_eq!(
            observer.summary(),
            settled,
            "the owner's terminal decision, not the provider's event, closes transport accounting"
        );
    }

    #[test]
    fn incomplete_and_cancelled_never_become_completed() {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(
            observer
                .push(b"event: response.in_progress\ndata: {}\n\n")
                .is_ok()
        );
        assert_eq!(
            observer.finish().response_state,
            ProviderResponseState::Incomplete
        );
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert_eq!(
            observer.cancel().response_state,
            ProviderResponseState::Cancelled
        );
    }

    #[test]
    fn a_disconnect_after_completion_keeps_the_terminal_state_and_usage() {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(observer.push(COMPLETED_STREAM).is_ok());
        let completed = observer.summary();
        assert_eq!(completed.response_state, ProviderResponseState::Completed);
        assert_eq!(completed.usage_status, UsageStatus::Final);
        let disconnected = observer.disconnect();
        assert_eq!(
            semantic_projection(&disconnected),
            semantic_projection(&completed)
        );
        let cancelled = observer.cancel();
        assert_eq!(
            semantic_projection(&cancelled),
            semantic_projection(&completed)
        );
    }

    #[test]
    fn a_disconnect_without_terminal_evidence_reports_the_disconnect() {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(
            observer
                .push(b"event: response.in_progress\ndata: {\"response\":{\"id\":\"resp_2\"}}\n\n")
                .is_ok()
        );
        let observation = observer.disconnect();
        assert_eq!(
            observation.response_state,
            ProviderResponseState::Disconnected
        );
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
        assert_eq!(observation.status, ObservationStatus::Partial);
        assert_eq!(observation.provider_response_id.as_deref(), Some("resp_2"));
    }

    #[test]
    fn a_failed_event_survives_a_later_disconnect() {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(
            observer
                .push(
                    b"event: response.failed\ndata: {\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"rate_limit_exceeded\"}}}\n\n"
                )
                .is_ok()
        );
        let observation = observer.disconnect();
        assert_eq!(observation.response_state, ProviderResponseState::Failed);
        assert_eq!(
            observation.error_code.as_deref(),
            Some("rate_limit_exceeded")
        );
    }

    #[test]
    fn an_oversized_streamed_string_is_dropped_and_marks_partial() {
        let long = "a".repeat(20 * 1024);
        let stream = format!(
            "event: response.completed\ndata: {{\"response\":{{\"id\":\"{long}\",\"model\":\"gpt\",\"status\":\"completed\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}}}\n\n"
        );
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(observer.push(stream.as_bytes()).is_ok());
        let observation = observer.finish();
        assert_eq!(observation.provider_response_id, None);
        assert_eq!(observation.model.as_deref(), Some("gpt"));
        assert_eq!(observation.status, ObservationStatus::Partial);
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
        assert_eq!(observation.usage_status, UsageStatus::Final);
    }

    #[test]
    fn only_the_top_level_usage_member_of_the_response_is_retained() {
        let stream = b"event: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"tools\":[{\"parameters\":{\"properties\":{\"usage\":{\"description\":\"CANARY\"}}}}],\"note\":\"usage\",\"usage\":{\"input_tokens\":5,\"output_tokens\":2,\"total_tokens\":7}}}\n\n";
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(observer.push(stream).is_ok());
        let observation = observer.finish();
        assert_eq!(
            observation.raw_usage.as_ref().map(AsRef::as_ref),
            Some(&br#"{"input_tokens":5,"output_tokens":2,"total_tokens":7}"#[..])
        );
        assert_eq!(observation.usage_status, UsageStatus::Final);
        assert_eq!(
            observation
                .normalized_usage
                .as_ref()
                .and_then(|usage| usage.total),
            Some(7)
        );
    }

    #[test]
    fn an_empty_usage_object_on_completion_is_unavailable_and_partial() {
        let stream =
            b"event: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"usage\":{}}}\n\n";
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(observer.push(stream).is_ok());
        let observation = observer.finish();
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
        assert_eq!(observation.status, ObservationStatus::Partial);
        assert_eq!(observation.response_state, ProviderResponseState::Completed);
    }

    #[test]
    fn duplicate_keys_in_one_event_are_not_interpreted() {
        let stream = b"event: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1},\"usage\":{\"input_tokens\":2}}}\n\n";
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(observer.push(stream).is_ok());
        let observation = observer.finish();
        assert!(observation.raw_usage.is_none());
        assert_eq!(observation.usage_status, UsageStatus::Unavailable);
        assert_eq!(observation.status, ObservationStatus::Partial);
    }

    #[test]
    fn timings_are_measured_and_ordered() {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(
            observer
                .push(b"event: response.created\ndata: {\"response\":{\"id\":\"resp_3\"}}\n\n")
                .is_ok()
        );
        std::thread::sleep(std::time::Duration::from_micros(1500));
        assert!(
            observer
                .push(b"event: response.output_text.delta\ndata: {\"delta\":\"x\"}\n\n")
                .is_ok()
        );
        std::thread::sleep(std::time::Duration::from_micros(1500));
        assert!(
            observer
                .push(
                    b"event: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n"
                )
                .is_ok()
        );
        let observation = observer.finish();
        let first_byte = observation.ttfb_us;
        let first_output = observation.ttft_us;
        let duration = observation.duration_us;
        assert!(first_byte.is_some(), "time to first byte must be measured");
        assert!(
            first_output > first_byte,
            "time to first token {first_output:?} must follow first byte {first_byte:?}"
        );
        assert!(
            duration > first_output,
            "duration {duration:?} must follow first token {first_output:?}"
        );
    }

    #[test]
    fn a_stream_without_semantic_output_reports_no_first_token_measurement() {
        let mut observer = StreamingObserver::new(ObservationLimits::default());
        assert!(
            observer
                .push(b"event: response.created\ndata: {}\n\n")
                .is_ok()
        );
        let observation = observer.finish();
        assert_eq!(observation.ttft_us, None);
        assert!(observation.ttfb_us.is_some());
        assert!(observation.duration_us.is_some());
    }
}
