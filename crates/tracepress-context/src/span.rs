//! Bounded raw JSON span indexing over the exact bytes of one observed request.
//!
//! The indexer is a single non-recursive scan of a shared byte slice. For every value it visits it
//! records a byte range that is exactly that JSON value: a string range includes its enclosing
//! quotes, an object or array range includes its enclosing braces or brackets, and a scalar range
//! is its literal text. No unescaping, normalisation, or re-encoding participates in span
//! computation, so the span stays usable as the authoritative structural identity of a block even
//! when the decoded value is ambiguous.
//!
//! Colliding member names are recorded, never resolved: every colliding member keeps its own
//! occurrence index and the index reports [`RawSpanIndex::duplicate_key_detected`], because a
//! JSON-Pointer path stops identifying a unique value while the raw spans still do.
//!
//! Every dimension of the scan is bounded by [`ContextAnalysisLimits`]. Crossing a bound stops the
//! scan, names the dimension through [`RawSpanIndex::limit_reached`], and keeps every value
//! already indexed inside the bound; nothing already completed is discarded because a later bound
//! was crossed. Values left open when the scan stopped are reported by
//! [`SpanNode::is_complete`] as incomplete rather than given an invented end.

use std::{collections::HashMap, fmt, str::Chars, time::Instant};

use thiserror::Error;
use tracepress_core::{
    ProcessingBudget, ProcessingBudgetAxis, ProcessingBudgetDecision, ProcessingCharge,
};

use crate::{
    BlockLocator, BoundedMetadataText, ContextAnalysisLimitField, ContextAnalysisLimits,
    ContextAnalysisStatus,
};

/// Request bytes one charged work unit covers.
///
/// A work unit is a coarse quantum of scanning, not a CPU-cycle count: it exists so the operator's
/// configured work budget also bounds analysis.
const BYTES_PER_WORK_UNIT: u64 = 4096;
/// Indexed values one charged work unit covers.
const VALUES_PER_WORK_UNIT: u64 = 64;
/// FNV-1a offset basis for bounded member-name fingerprints.
const NAME_HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a prime for bounded member-name fingerprints.
const NAME_HASH_PRIME: u64 = 0x0000_0100_0000_01b3;
/// First code unit of the UTF-16 high surrogate range.
const HIGH_SURROGATE_START: u16 = 0xD800;
/// First code unit of the UTF-16 low surrogate range.
const LOW_SURROGATE_START: u16 = 0xDC00;
/// One past the last code unit of the UTF-16 surrogate range.
const SURROGATE_END: u16 = 0xE000;
/// Bytes a surrogate code unit is charged against a decode bound.
const SURROGATE_ENCODED_BYTES: usize = 3;

/// A byte range inside the original request.
///
/// The range is half-open and is never resolved against the request bytes implicitly: slicing goes
/// through [`RawSpan::slice`], which refuses a range the caller's bytes do not cover instead of
/// panicking.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RawSpan {
    start: u64,
    end: u64,
}

impl RawSpan {
    /// Creates a byte range, refusing an inverted one.
    #[must_use]
    pub const fn new(start: u64, end: u64) -> Option<Self> {
        if start > end {
            return None;
        }
        Some(Self { start, end })
    }

    /// Returns the first byte of the range.
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Returns one past the last byte of the range.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.end
    }

    /// Returns the length of the range in bytes.
    #[must_use]
    pub const fn len_bytes(self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Returns whether the range covers no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns whether this range covers all of `inner`.
    #[must_use]
    pub const fn covers(self, inner: Self) -> bool {
        self.start <= inner.start && inner.end <= self.end
    }

    /// Returns whether this range shares a byte with `other`.
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Borrows the bytes of this range, refusing a range the bytes do not cover.
    #[must_use]
    pub fn slice(self, request: &[u8]) -> Option<&[u8]> {
        let start = usize::try_from(self.start).ok()?;
        let end = usize::try_from(self.end).ok()?;
        request.get(start..end)
    }
}

/// The kind of one JSON value.
#[allow(
    clippy::exhaustive_enums,
    reason = "JSON defines exactly these six value kinds"
)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JsonValueKind {
    /// An object.
    Object,
    /// An array.
    Array,
    /// A string.
    String,
    /// A number.
    Number,
    /// `true` or `false`.
    Boolean,
    /// `null`.
    Null,
}

impl JsonValueKind {
    /// Returns whether values of this kind carry members or elements.
    #[must_use]
    pub const fn is_container(self) -> bool {
        matches!(self, Self::Object | Self::Array)
    }
}

/// Identity of one indexed value inside one [`RawSpanIndex`].
///
/// Identities are dense and follow document order, so [`SpanNodeId::index`] doubles as the
/// document-order position of the value within its index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpanNodeId(u32);

impl SpanNodeId {
    /// Returns the document-order position of the value.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// One JSON value the indexer visited, positioned in the original request bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpanNode {
    id: SpanNodeId,
    parent: Option<SpanNodeId>,
    first_child: Option<SpanNodeId>,
    last_child: Option<SpanNodeId>,
    next_sibling: Option<SpanNodeId>,
    locator: BlockLocator,
    kind: JsonValueKind,
    depth: u32,
    name: Option<RawSpan>,
    array_index: Option<u32>,
    child_count: u32,
    complete: bool,
    duplicate_key_detected: bool,
}

impl SpanNode {
    /// Returns the identity of this value.
    #[must_use]
    pub const fn id(&self) -> SpanNodeId {
        self.id
    }

    /// Returns the enclosing value, absent for the document root.
    #[must_use]
    pub const fn parent(&self) -> Option<SpanNodeId> {
        self.parent
    }

    /// Returns the kind of this value.
    #[must_use]
    pub const fn kind(&self) -> JsonValueKind {
        self.kind
    }

    /// Returns the nesting depth of this value, counting the document root as one.
    #[must_use]
    pub const fn depth(&self) -> u32 {
        self.depth
    }

    /// Returns the byte range that is exactly this JSON value.
    #[must_use]
    pub const fn span(&self) -> RawSpan {
        RawSpan {
            start: self.locator.raw_value_start,
            end: self.locator.raw_value_end,
        }
    }

    /// Returns the bytes this value occupies in the original request.
    #[must_use]
    pub const fn raw_bytes(&self) -> u64 {
        self.span().len_bytes()
    }

    /// Returns the position of this value, as the durable locator of a block.
    #[must_use]
    pub const fn locator(&self) -> &BlockLocator {
        &self.locator
    }

    /// Returns the JSON-Pointer-shaped path of this value.
    ///
    /// The path is a bounded convenience and is never unique: colliding member names give two
    /// values the same path, which is why [`SpanNode::occurrence`] and the raw span exist.
    #[must_use]
    pub const fn semantic_path(&self) -> &BoundedMetadataText {
        &self.locator.semantic_path
    }

    /// Returns how many same-named siblings precede this member.
    #[must_use]
    pub const fn occurrence(&self) -> u32 {
        self.locator.occurrence
    }

    /// Returns the byte range of the member name, including its quotes.
    ///
    /// Absent for the document root and for an array element, neither of which is named.
    #[must_use]
    pub const fn name_span(&self) -> Option<RawSpan> {
        self.name
    }

    /// Returns the position of this element within its enclosing array.
    #[must_use]
    pub const fn array_index(&self) -> Option<u32> {
        self.array_index
    }

    /// Returns the byte range between the quotes of a complete string value.
    #[must_use]
    pub const fn content_span(&self) -> Option<RawSpan> {
        if !self.complete || !matches!(self.kind, JsonValueKind::String) {
            return None;
        }
        inner_span(self.span())
    }

    /// Returns how many members or elements of this value were indexed.
    #[must_use]
    pub const fn child_count(&self) -> u32 {
        self.child_count
    }

    /// Returns whether the span exactly delimits a complete JSON value.
    ///
    /// It is `false` only when a bound stopped the scan inside this value or the analysed window
    /// cut it, in which case the span ends where scanning stopped rather than at a delimiter the
    /// indexer never saw.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// Returns whether this object has colliding member names.
    #[must_use]
    pub const fn duplicate_key_detected(&self) -> bool {
        self.duplicate_key_detected
    }

    /// Returns whether the decoded member name of this value equals `name`.
    ///
    /// Comparison is over decoded characters, so an escaped name matches its plain spelling, and
    /// it never allocates or decodes more than `name` requires.
    #[must_use]
    pub fn name_equals(&self, request: &[u8], name: &str) -> bool {
        let Some(text) = self.name.and_then(|span| quoted_text(request, span)) else {
            return false;
        };
        let mut decoded = JsonStringUnits::new(text);
        let mut expected = name.chars();
        loop {
            match (decoded.next(), expected.next()) {
                (None, None) => return true,
                (Some(Ok(StringUnit::Character(observed))), Some(wanted)) if observed == wanted => {
                }
                _ => return false,
            }
        }
    }
}

/// Members or elements of one indexed value, in document order.
#[derive(Clone, Debug)]
pub struct SpanNodeChildren<'index> {
    index: &'index RawSpanIndex,
    next: Option<SpanNodeId>,
}

impl<'index> Iterator for SpanNodeChildren<'index> {
    type Item = &'index SpanNode;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.index.node(self.next?)?;
        self.next = node.next_sibling;
        Some(node)
    }
}

/// A structural index of every JSON value the indexer visited in one request.
///
/// The index retains no request bytes: every value is a position, so reading content always needs
/// the caller's own shared slice of the original request.
pub struct RawSpanIndex {
    nodes: Vec<SpanNode>,
    status: ContextAnalysisStatus,
    limit_reached: Option<ContextAnalysisLimitField>,
    analyzed_bytes: u64,
    skipped_bytes: u64,
    duplicate_key_detected: bool,
}

impl RawSpanIndex {
    /// Indexes the spans of one request against a monotonic clock.
    ///
    /// The request is borrowed and never mutated.
    #[must_use]
    pub fn build(request: &[u8], limits: ContextAnalysisLimits) -> Self {
        Self::build_with_clock(request, limits, &MonotonicClock::started_now())
    }

    /// Indexes the spans of one request against a caller-supplied clock.
    ///
    /// The clock is sampled before publishing values and periodically inside long byte and name
    /// scans, so no single large value can hide wall-clock exhaustion until after completion.
    #[must_use]
    pub fn build_with_clock<ClockType>(
        request: &[u8],
        limits: ContextAnalysisLimits,
        clock: &ClockType,
    ) -> Self
    where
        ClockType: AnalysisClock,
    {
        let request_bytes = as_u64(request.len());
        let bound = limits.max_analyzed_bytes.get();
        let truncated = request.len() > bound;
        let mut window = if truncated {
            request.get(..bound).unwrap_or(request)
        } else {
            request
        };
        match std::str::from_utf8(window) {
            Ok(_) => {}
            // A window cut by the byte bound may end inside a multi-byte character; that is the
            // bound speaking, not malformed input, so the analysed window shrinks to the last
            // complete character instead of condemning the request.
            Err(error) if truncated && error.error_len().is_none() => {
                window = window.get(..error.valid_up_to()).unwrap_or(&[]);
            }
            Err(_) => return Self::malformed(request_bytes),
        }
        if window.contains(&0) {
            return Self::malformed(request_bytes);
        }

        let mut scanner = Scanner::new(window, limits, clock);
        let stop = scanner.run().err();
        scanner.finish(ScanOutcome {
            request_bytes,
            window_truncated: truncated,
            stop,
        })
    }

    /// Returns the outcome of the scan.
    #[must_use]
    pub const fn status(&self) -> ContextAnalysisStatus {
        self.status
    }

    /// Returns the bound that stopped the scan, absent when none was crossed.
    #[must_use]
    pub const fn limit_reached(&self) -> Option<ContextAnalysisLimitField> {
        self.limit_reached
    }

    /// Returns whether any indexed object has colliding member names.
    #[must_use]
    pub const fn duplicate_key_detected(&self) -> bool {
        self.duplicate_key_detected
    }

    /// Returns the request bytes the scan structurally interpreted.
    #[must_use]
    pub const fn analyzed_bytes(&self) -> u64 {
        self.analyzed_bytes
    }

    /// Returns the request bytes the scan never interpreted.
    ///
    /// The request is forwarded in full either way; these bytes are the honest difference between
    /// what was sent and what was analysed.
    #[must_use]
    pub const fn skipped_bytes(&self) -> u64 {
        self.skipped_bytes
    }

    /// Returns every indexed value in document order.
    #[must_use]
    pub fn nodes(&self) -> &[SpanNode] {
        &self.nodes
    }

    /// Returns how many values were indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns whether no value was indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Returns the root value of the document, absent when nothing was indexed.
    #[must_use]
    pub fn root(&self) -> Option<&SpanNode> {
        self.nodes.first()
    }

    /// Returns one indexed value.
    #[must_use]
    pub fn node(&self, id: SpanNodeId) -> Option<&SpanNode> {
        self.nodes.get(usize::try_from(id.0).ok()?)
    }

    /// Returns the members or elements of one indexed value, in document order.
    #[must_use]
    pub fn children(&self, id: SpanNodeId) -> SpanNodeChildren<'_> {
        SpanNodeChildren {
            index: self,
            next: self.node(id).and_then(|node| node.first_child),
        }
    }

    /// Returns the first member of an object whose decoded name equals `name`.
    ///
    /// Colliding members are reachable through [`RawSpanIndex::children`] with their own
    /// occurrence indexes; this accessor deliberately resolves nothing on their behalf.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the index retains no request bytes, so a name lookup needs the caller's slice next to the object and the name"
    )]
    pub fn member(&self, request: &[u8], object: SpanNodeId, name: &str) -> Option<&SpanNode> {
        self.children(object)
            .find(|node| node.name_equals(request, name))
    }

    const fn malformed(request_bytes: u64) -> Self {
        Self {
            nodes: Vec::new(),
            status: ContextAnalysisStatus::Malformed,
            limit_reached: None,
            analyzed_bytes: 0,
            skipped_bytes: request_bytes,
            duplicate_key_detected: false,
        }
    }
}

impl fmt::Debug for RawSpanIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawSpanIndex")
            .field("status", &self.status)
            .field("limit_reached", &self.limit_reached)
            .field("indexed_values", &self.nodes.len())
            .field("analyzed_bytes", &self.analyzed_bytes)
            .field("skipped_bytes", &self.skipped_bytes)
            .field("duplicate_key_detected", &self.duplicate_key_detected)
            .finish()
    }
}

/// A monotonic elapsed-time source the indexer samples while scanning.
///
/// It is a parameter rather than a direct clock read so the wall-clock bound is deterministically
/// observable and so an analysis can share the clock its caller already started.
pub trait AnalysisClock {
    /// Returns the milliseconds elapsed since the analysis started.
    fn elapsed_ms(&self) -> u64;
}

/// The elapsed-time source of one analysis, started from the process monotonic clock.
#[derive(Clone, Copy, Debug)]
pub struct MonotonicClock {
    started: Instant,
}

impl MonotonicClock {
    /// Starts measuring now.
    #[must_use]
    pub fn started_now() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::started_now()
    }
}

impl AnalysisClock for MonotonicClock {
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Failure to decode one JSON string span.
///
/// No variant carries decoded characters, so a rejection stays inside the privacy boundary.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum JsonStringDecodeError {
    /// The span does not delimit a quoted JSON string.
    #[error("span does not delimit a JSON string")]
    NotAJsonString,
    /// The span bytes are not valid UTF-8.
    #[error("JSON string bytes are not valid UTF-8")]
    NotUtf8,
    /// The string carries an escape sequence JSON does not define.
    #[error("JSON string carries an invalid escape sequence")]
    InvalidEscape,
    /// The string carries an unescaped control character.
    #[error("JSON string carries an unescaped control character")]
    ControlCharacter,
    /// The string carries a surrogate code unit without its pair, which decodes to no character.
    #[error("JSON string carries an unpaired surrogate")]
    UnpairedSurrogate,
    /// Decoding would have exceeded the configured string inspection bound.
    #[error("decoded JSON string exceeds the {maximum_bytes} byte inspection bound")]
    AboveInspectionBound {
        /// The configured bound in bytes.
        maximum_bytes: u64,
    },
}

/// Decodes one JSON string span to its characters, bounded by the inspection limit.
///
/// The span must be the exact span of a JSON string, quotes included, as reported by
/// [`SpanNode::span`]. Decoding is refused rather than truncated when the decoded value would
/// exceed `max_string_bytes_inspected`, because a truncated decode is not an equivalence: a caller
/// that cannot tell the difference would fingerprint two different strings identically.
///
/// # Errors
/// Returns [`JsonStringDecodeError`] when the span is not a JSON string, is not UTF-8, carries an
/// invalid escape, an unescaped control character, or an unpaired surrogate, or when the decoded
/// value does not fit the inspection bound.
pub fn decode_json_string(
    request: &[u8],
    span: RawSpan,
    limits: ContextAnalysisLimits,
) -> Result<String, JsonStringDecodeError> {
    let text = quoted_text(request, span).ok_or(JsonStringDecodeError::NotAJsonString)?;
    let maximum = limits.max_string_bytes_inspected.get();
    let mut decoded = String::with_capacity(text.len().min(maximum));
    for unit in JsonStringUnits::new(text) {
        match unit {
            Ok(StringUnit::Character(character)) => {
                if decoded.len().saturating_add(character.len_utf8()) > maximum {
                    return Err(JsonStringDecodeError::AboveInspectionBound {
                        maximum_bytes: as_u64(maximum),
                    });
                }
                decoded.push(character);
            }
            Ok(StringUnit::UnpairedSurrogate(_)) => {
                return Err(JsonStringDecodeError::UnpairedSurrogate);
            }
            Err(StringFault::InvalidEscape) => return Err(JsonStringDecodeError::InvalidEscape),
            Err(StringFault::ControlCharacter) => {
                return Err(JsonStringDecodeError::ControlCharacter);
            }
        }
    }
    Ok(decoded)
}

/// The reason one scan stopped early.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stop {
    /// The bytes are not one valid JSON document.
    Malformed(MalformedReason),
    /// One configured analysis bound was crossed.
    Limit(ContextAnalysisLimitField),
}

/// How the bytes failed to be one valid JSON document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MalformedReason {
    /// The document ended while a value was still expected.
    UnexpectedEnd,
    /// A byte appeared where the JSON grammar does not allow it.
    Syntax,
}

/// Whether reading a value entered a container the scan must now step through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Descent {
    /// A container was entered and owns its own path and name bookkeeping.
    Entered,
    /// The value is finished.
    Completed,
}

/// The kind of container one open frame is stepping through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Container {
    Object,
    Array,
}

/// One open container of the explicit scan stack.
#[derive(Clone, Copy, Debug)]
struct Frame {
    node: SpanNodeId,
    kind: Container,
    children: u32,
    depth: u32,
    path_base: usize,
}

/// Everything a value needs to know about its position before it is indexed.
///
/// `start` is the first byte of the value, which every caller reads off the cursor after
/// whitespace has been skipped.
#[derive(Clone, Copy, Debug)]
struct ValueContext {
    parent: Option<SpanNodeId>,
    name: Option<RawSpan>,
    occurrence: u32,
    array_index: Option<u32>,
    depth: u32,
    path_base: usize,
    start: usize,
}

/// One JSON literal spelling and the kind it denotes.
#[derive(Clone, Copy, Debug)]
struct Literal {
    word: &'static [u8],
    kind: JsonValueKind,
}

impl Literal {
    const TRUE: Self = Self {
        word: b"true",
        kind: JsonValueKind::Boolean,
    };
    const FALSE: Self = Self {
        word: b"false",
        kind: JsonValueKind::Boolean,
    };
    const NULL: Self = Self {
        word: b"null",
        kind: JsonValueKind::Null,
    };
}

/// What one finished scan knows about the request it did not fully analyse.
#[derive(Clone, Copy, Debug)]
struct ScanOutcome {
    request_bytes: u64,
    window_truncated: bool,
    stop: Option<Stop>,
}

/// One bounded member-name fingerprint entry.
#[derive(Clone, Copy, Debug)]
struct NameEntry {
    content: RawSpan,
    occurrences: u32,
}

/// The bounded, non-recursive JSON scan of one analysed window.
struct Scanner<'run, ClockType> {
    window: &'run [u8],
    clock: &'run ClockType,
    cursor: usize,
    max_blocks: usize,
    max_depth: u32,
    string_bound: usize,
    budget: ProcessingBudget,
    charged_ms: u64,
    reserved_work_units: u64,
    observed_bytes: u64,
    observed_values: u64,
    nodes: Vec<SpanNode>,
    frames: Vec<Frame>,
    names: HashMap<(SpanNodeId, u64), Vec<NameEntry>>,
    path: String,
    duplicate_key_detected: bool,
}

impl<'run, ClockType> Scanner<'run, ClockType>
where
    ClockType: AnalysisClock,
{
    fn new(window: &'run [u8], limits: ContextAnalysisLimits, clock: &'run ClockType) -> Self {
        let max_depth = u32::try_from(limits.max_json_depth.get()).unwrap_or(u32::MAX);
        Self {
            window,
            clock,
            cursor: 0,
            max_blocks: limits.max_blocks.get(),
            max_depth,
            string_bound: limits.max_string_bytes_inspected.get(),
            budget: limits.processing_budget(),
            charged_ms: 0,
            reserved_work_units: 0,
            observed_bytes: 0,
            observed_values: 0,
            nodes: Vec::new(),
            frames: Vec::with_capacity(usize::try_from(max_depth.min(64)).unwrap_or(16)),
            names: HashMap::with_capacity(limits.max_blocks.get()),
            path: String::new(),
            duplicate_key_detected: false,
        }
    }

    fn run(&mut self) -> Result<(), Stop> {
        self.skip_whitespace()?;
        let root = ValueContext {
            parent: None,
            name: None,
            occurrence: 0,
            array_index: None,
            depth: 1,
            path_base: 0,
            start: self.cursor,
        };
        let _descent: Descent = self.read_value(root)?;
        while !self.frames.is_empty() {
            self.step()?;
        }
        self.skip_whitespace()?;
        if self.cursor < self.window.len() {
            return Err(Stop::Malformed(MalformedReason::Syntax));
        }
        Ok(())
    }

    fn step(&mut self) -> Result<(), Stop> {
        let Some(index) = self.frames.len().checked_sub(1) else {
            return Ok(());
        };
        let Some(frame) = self.frames.get(index).copied() else {
            return Ok(());
        };
        match frame.kind {
            Container::Object => self.step_object(index, frame),
            Container::Array => self.step_array(index, frame),
        }
    }

    fn step_object(&mut self, index: usize, frame: Frame) -> Result<(), Stop> {
        self.skip_whitespace()?;
        match self.peek() {
            None => return Err(Stop::Malformed(MalformedReason::UnexpectedEnd)),
            Some(b'}') => {
                self.advance(1)?;
                self.close_frame();
                return self.poll_clock();
            }
            Some(b',') if frame.children > 0 => {
                self.advance(1)?;
                self.skip_whitespace()?;
            }
            Some(_) if frame.children == 0 => {}
            Some(_) => return Err(Stop::Malformed(MalformedReason::Syntax)),
        }
        if self.peek() != Some(b'"') {
            return Err(self.unexpected());
        }
        let name = self.scan_string(None)?;
        self.skip_whitespace()?;
        match self.peek() {
            Some(b':') => self.advance(1)?,
            Some(_) => return Err(Stop::Malformed(MalformedReason::Syntax)),
            None => return Err(Stop::Malformed(MalformedReason::UnexpectedEnd)),
        }
        self.skip_whitespace()?;
        let occurrence = self.record_name(frame.node, name)?;
        let path_base = self.path.len();
        self.push_name_token(name)?;
        let context = ValueContext {
            parent: Some(frame.node),
            name: Some(name),
            occurrence,
            array_index: None,
            depth: frame.depth.saturating_add(1),
            path_base,
            start: self.cursor,
        };
        if matches!(self.read_value(context)?, Descent::Completed) {
            self.rewind_path(path_base);
        }
        self.count_child(index);
        self.poll_clock()
    }

    fn step_array(&mut self, index: usize, frame: Frame) -> Result<(), Stop> {
        self.skip_whitespace()?;
        match self.peek() {
            None => return Err(Stop::Malformed(MalformedReason::UnexpectedEnd)),
            Some(b']') => {
                self.advance(1)?;
                self.close_frame();
                return self.poll_clock();
            }
            Some(b',') if frame.children > 0 => {
                self.advance(1)?;
                self.skip_whitespace()?;
            }
            Some(_) if frame.children == 0 => {}
            Some(_) => return Err(Stop::Malformed(MalformedReason::Syntax)),
        }
        let path_base = self.path.len();
        self.path.push('/');
        push_decimal(&mut self.path, frame.children);
        let context = ValueContext {
            parent: Some(frame.node),
            name: None,
            occurrence: 0,
            array_index: Some(frame.children),
            depth: frame.depth.saturating_add(1),
            path_base,
            start: self.cursor,
        };
        if matches!(self.read_value(context)?, Descent::Completed) {
            self.rewind_path(path_base);
        }
        self.count_child(index);
        self.poll_clock()
    }

    fn read_value(&mut self, context: ValueContext) -> Result<Descent, Stop> {
        if context.depth > self.max_depth {
            return Err(Stop::Limit(ContextAnalysisLimitField::JsonDepth));
        }
        let Some(byte) = self.peek() else {
            return Err(Stop::Malformed(MalformedReason::UnexpectedEnd));
        };
        match byte {
            b'{' | b'[' => {
                let (kind, container) = if byte == b'{' {
                    (JsonValueKind::Object, Container::Object)
                } else {
                    (JsonValueKind::Array, Container::Array)
                };
                self.advance(1)?;
                let node = self.push_node(context, kind)?;
                self.frames.push(Frame {
                    node,
                    kind: container,
                    children: 0,
                    depth: context.depth,
                    path_base: context.path_base,
                });
                Ok(Descent::Entered)
            }
            b'"' => {
                let node = self.push_node(context, JsonValueKind::String)?;
                let span = self.scan_string(Some(node))?;
                self.finish_node(node, span.end);
                Ok(Descent::Completed)
            }
            b't' => self.read_literal(context, Literal::TRUE),
            b'f' => self.read_literal(context, Literal::FALSE),
            b'n' => self.read_literal(context, Literal::NULL),
            b'-' | b'0'..=b'9' => {
                self.scan_number()?;
                let node = self.push_node(context, JsonValueKind::Number)?;
                self.finish_node(node, as_u64(self.cursor));
                Ok(Descent::Completed)
            }
            _ => Err(Stop::Malformed(MalformedReason::Syntax)),
        }
    }

    fn read_literal(&mut self, context: ValueContext, literal: Literal) -> Result<Descent, Stop> {
        let end = self.cursor.saturating_add(literal.word.len());
        match self.window.get(self.cursor..end) {
            Some(slice) if slice == literal.word => {}
            Some(_) => return Err(Stop::Malformed(MalformedReason::Syntax)),
            None => return Err(Stop::Malformed(MalformedReason::UnexpectedEnd)),
        }
        self.advance(literal.word.len())?;
        let node = self.push_node(context, literal.kind)?;
        self.finish_node(node, as_u64(end));
        Ok(Descent::Completed)
    }

    /// Scans one string, returning the span that includes both quotes.
    ///
    /// When `node` is present its observed end advances with the cursor and it remains incomplete
    /// until the closing quote has been validated.
    fn scan_string(&mut self, node: Option<SpanNodeId>) -> Result<RawSpan, Stop> {
        let start = self.cursor;
        self.advance_string(node, 1)?;
        loop {
            let Some(byte) = self.peek() else {
                return Err(Stop::Malformed(MalformedReason::UnexpectedEnd));
            };
            self.advance_string(node, 1)?;
            match byte {
                b'"' => break,
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(Stop::Malformed(MalformedReason::UnexpectedEnd));
                    };
                    self.advance_string(node, 1)?;
                    match escape {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                        b'u' => {
                            for _ in 0_u8..4 {
                                let Some(digit) = self.peek() else {
                                    return Err(Stop::Malformed(MalformedReason::UnexpectedEnd));
                                };
                                if !digit.is_ascii_hexdigit() {
                                    return Err(Stop::Malformed(MalformedReason::Syntax));
                                }
                                self.advance_string(node, 1)?;
                            }
                        }
                        _ => return Err(Stop::Malformed(MalformedReason::Syntax)),
                    }
                }
                0x00..=0x1f => return Err(Stop::Malformed(MalformedReason::Syntax)),
                _ => {}
            }
        }
        let end = self.cursor;
        RawSpan::new(as_u64(start), as_u64(end)).ok_or(Stop::Malformed(MalformedReason::Syntax))
    }

    fn advance_string(&mut self, node: Option<SpanNodeId>, bytes: usize) -> Result<(), Stop> {
        self.advance(bytes)?;
        let end = as_u64(self.cursor);
        if let Some(id) = node {
            if let Some(value) = self.node_mut(id) {
                value.locator.raw_value_end = end;
            }
        }
        Ok(())
    }

    fn scan_number(&mut self) -> Result<(), Stop> {
        if self.peek() == Some(b'-') {
            self.advance(1)?;
        }
        match self.peek() {
            Some(b'0') => self.advance(1)?,
            Some(b'1'..=b'9') => {
                self.advance(1)?;
                self.skip_digits()?;
            }
            Some(_) => return Err(Stop::Malformed(MalformedReason::Syntax)),
            None => return Err(Stop::Malformed(MalformedReason::UnexpectedEnd)),
        }
        if self.peek() == Some(b'.') {
            self.advance(1)?;
            self.require_digit()?;
            self.skip_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.advance(1)?;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.advance(1)?;
            }
            self.require_digit()?;
            self.skip_digits()?;
        }
        Ok(())
    }

    fn require_digit(&mut self) -> Result<(), Stop> {
        match self.peek() {
            Some(byte) if byte.is_ascii_digit() => self.advance(1),
            Some(_) => Err(Stop::Malformed(MalformedReason::Syntax)),
            None => Err(Stop::Malformed(MalformedReason::UnexpectedEnd)),
        }
    }

    fn skip_digits(&mut self) -> Result<(), Stop> {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.advance(1)?;
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) -> Result<(), Stop> {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.advance(1)?;
        }
        Ok(())
    }

    fn peek(&self) -> Option<u8> {
        self.window.get(self.cursor).copied()
    }

    fn advance(&mut self, bytes: usize) -> Result<(), Stop> {
        let next_cursor = self.cursor.saturating_add(bytes).min(self.window.len());
        let consumed = as_u64(next_cursor.saturating_sub(self.cursor));
        let next_bytes = self.observed_bytes.saturating_add(consumed);
        self.reserve_work(next_bytes, self.observed_values)?;
        self.cursor = next_cursor;
        self.observed_bytes = next_bytes;
        Ok(())
    }

    fn unexpected(&self) -> Stop {
        if self.peek().is_some() {
            Stop::Malformed(MalformedReason::Syntax)
        } else {
            Stop::Malformed(MalformedReason::UnexpectedEnd)
        }
    }

    fn push_node(
        &mut self,
        context: ValueContext,
        kind: JsonValueKind,
    ) -> Result<SpanNodeId, Stop> {
        if self.nodes.len() >= self.max_blocks {
            return Err(Stop::Limit(ContextAnalysisLimitField::Blocks));
        }
        self.poll_clock()?;
        self.reserve_work(self.observed_bytes, self.observed_values.saturating_add(1))?;
        let ordinal = u32::try_from(self.nodes.len())
            .map_err(|_error| Stop::Limit(ContextAnalysisLimitField::Blocks))?;
        let id = SpanNodeId(ordinal);
        let start_byte = as_u64(context.start);
        self.nodes.push(SpanNode {
            id,
            parent: context.parent,
            first_child: None,
            last_child: None,
            next_sibling: None,
            locator: BlockLocator {
                semantic_path: BoundedMetadataText::semantic_path(&self.path),
                raw_value_start: start_byte,
                raw_value_end: start_byte,
                occurrence: context.occurrence,
            },
            kind,
            depth: context.depth,
            name: context.name,
            array_index: context.array_index,
            child_count: 0,
            complete: false,
            duplicate_key_detected: false,
        });
        self.observed_values = self.observed_values.saturating_add(1);
        if let Some(parent) = context.parent {
            self.link_child(parent, id);
        }
        Ok(id)
    }

    fn link_child(&mut self, parent: SpanNodeId, child: SpanNodeId) {
        let last_child = match self.node_mut(parent) {
            Some(node) => {
                let previous = node.last_child;
                if previous.is_none() {
                    node.first_child = Some(child);
                }
                node.last_child = Some(child);
                node.child_count = node.child_count.saturating_add(1);
                previous
            }
            None => return,
        };
        if let Some(previous) = last_child {
            if let Some(node) = self.node_mut(previous) {
                node.next_sibling = Some(child);
            }
        }
    }

    fn count_child(&mut self, frame_index: usize) {
        if let Some(frame) = self.frames.get_mut(frame_index) {
            frame.children = frame.children.saturating_add(1);
        }
    }

    fn finish_node(&mut self, id: SpanNodeId, end: u64) {
        if let Some(node) = self.node_mut(id) {
            node.locator.raw_value_end = end;
            node.complete = true;
        }
    }

    fn node_mut(&mut self, id: SpanNodeId) -> Option<&mut SpanNode> {
        let index = usize::try_from(id.0).ok()?;
        self.nodes.get_mut(index)
    }

    fn close_frame(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let end = as_u64(self.cursor);
        self.finish_node(frame.node, end);
        self.rewind_path(frame.path_base);
    }

    /// Records one member name and returns how many same-named members precede it.
    ///
    /// The randomized table makes distinct-name insertion expected O(1). A digest collision is
    /// resolved by bounded decoded equality, and equal spellings share one occurrence counter.
    fn record_name(&mut self, object: SpanNodeId, name: RawSpan) -> Result<u32, Stop> {
        let Some(content) = inner_span(name) else {
            return Ok(0);
        };
        let hash = self.hash_name(content)?;
        let key = (object, hash);
        let entries = self.names.get(&key).cloned().unwrap_or_default();
        let mut matched = None;
        for (position, entry) in entries.iter().enumerate() {
            if self.names_equal(entry.content, content)? {
                matched = Some((position, entry.occurrences));
                break;
            }
        }
        let occurrence = matched.map_or(0, |(_, count)| count);
        let bucket = self.names.entry(key).or_default();
        if let Some((position, count)) = matched {
            if let Some(entry) = bucket.get_mut(position) {
                entry.occurrences = count.saturating_add(1);
            }
            self.duplicate_key_detected = true;
            if let Some(node) = self.node_mut(object) {
                node.duplicate_key_detected = true;
            }
        } else {
            bucket.push(NameEntry {
                content,
                occurrences: 1,
            });
        }
        Ok(occurrence)
    }

    fn hash_name(&mut self, content: RawSpan) -> Result<u64, Stop> {
        let mut hash = NAME_HASH_OFFSET;
        let Some(text) = content
            .slice(self.window)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        else {
            return Ok(hash);
        };
        let mut decoded = 0_usize;
        for (position, unit) in JsonStringUnits::new(text).enumerate() {
            if position % 256 == 0 {
                self.poll_clock()?;
            }
            let Ok(unit) = unit else {
                break;
            };
            let next = decoded.saturating_add(unit.encoded_len());
            if next > self.string_bound {
                break;
            }
            decoded = next;
            hash = mix(hash, unit.tagged_code());
        }
        Ok(hash)
    }

    fn names_equal(&mut self, left: RawSpan, right: RawSpan) -> Result<bool, Stop> {
        if left == right {
            return Ok(true);
        }
        let (Some(left_text), Some(right_text)) = (
            left.slice(self.window)
                .and_then(|bytes| std::str::from_utf8(bytes).ok()),
            right
                .slice(self.window)
                .and_then(|bytes| std::str::from_utf8(bytes).ok()),
        ) else {
            return Ok(false);
        };
        let mut left_units = JsonStringUnits::new(left_text);
        let mut right_units = JsonStringUnits::new(right_text);
        let mut decoded = 0_usize;
        let mut comparisons = 0_usize;
        loop {
            if comparisons % 256 == 0 {
                self.poll_clock()?;
            }
            comparisons = comparisons.saturating_add(1);
            match (left_units.next(), right_units.next()) {
                (None, None) => return Ok(true),
                (Some(Ok(left_unit)), Some(Ok(right_unit))) if left_unit == right_unit => {
                    decoded = decoded.saturating_add(left_unit.encoded_len());
                    if decoded >= self.string_bound {
                        return Ok(true);
                    }
                }
                _ => return Ok(false),
            }
        }
    }

    /// Appends the JSON-Pointer token of one member name to the working path.
    fn push_name_token(&mut self, name: RawSpan) -> Result<(), Stop> {
        self.path.push('/');
        let Some(content) = inner_span(name) else {
            return Ok(());
        };
        let Some(text) = content
            .slice(self.window)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        else {
            return Ok(());
        };
        let mut decoded = 0_usize;
        for (position, unit) in JsonStringUnits::new(text).enumerate() {
            if position % 256 == 0 {
                self.poll_clock()?;
            }
            let Ok(unit) = unit else {
                break;
            };
            let character = match unit {
                StringUnit::Character(character) => character,
                StringUnit::UnpairedSurrogate(_) => char::REPLACEMENT_CHARACTER,
            };
            let next = decoded.saturating_add(character.len_utf8());
            if next > self.string_bound {
                break;
            }
            decoded = next;
            match character {
                '~' => self.path.push_str("~0"),
                '/' => self.path.push_str("~1"),
                _ => self.path.push(character),
            }
        }
        Ok(())
    }

    fn rewind_path(&mut self, length: usize) {
        if length <= self.path.len() && self.path.is_char_boundary(length) {
            self.path.truncate(length);
        }
    }

    fn reserve_work(&mut self, bytes: u64, values: u64) -> Result<(), Stop> {
        let byte_units = bytes.saturating_add(BYTES_PER_WORK_UNIT - 1) / BYTES_PER_WORK_UNIT;
        let value_units = values.saturating_add(VALUES_PER_WORK_UNIT - 1) / VALUES_PER_WORK_UNIT;
        let required = byte_units.max(value_units);
        let additional = required.saturating_sub(self.reserved_work_units);
        if additional == 0 {
            return Ok(());
        }
        self.charge(additional)?;
        self.reserved_work_units = required;
        Ok(())
    }

    fn poll_clock(&mut self) -> Result<(), Stop> {
        self.charge(0)
    }

    fn charge(&mut self, work_units: u64) -> Result<(), Stop> {
        let observed_ms = self.clock.elapsed_ms();
        let elapsed_ms = observed_ms.saturating_sub(self.charged_ms);
        match self
            .budget
            .charge(ProcessingCharge::new(elapsed_ms, work_units))
        {
            ProcessingBudgetDecision::Continue { .. } => {
                self.charged_ms = observed_ms;
                Ok(())
            }
            ProcessingBudgetDecision::Exhausted { axis, .. } => Err(Stop::Limit(match axis {
                ProcessingBudgetAxis::Time => ContextAnalysisLimitField::AnalysisWallTimeMs,
                ProcessingBudgetAxis::CpuWork => ContextAnalysisLimitField::AnalysisWorkUnits,
            })),
        }
    }

    fn finish(self, outcome: ScanOutcome) -> RawSpanIndex {
        let mut nodes = self.nodes;
        let analyzed_bytes = as_u64(self.cursor);
        // A container the scan never closed still holds the bytes it was given: its span ends
        // where scanning stopped, which keeps every child it did index inside it, and it stays
        // incomplete because the closing delimiter was never seen.
        for frame in &self.frames {
            if let Some(node) = usize::try_from(frame.node.index())
                .ok()
                .and_then(|position| nodes.get_mut(position))
            {
                node.locator.raw_value_end = analyzed_bytes;
            }
        }
        if outcome.window_truncated {
            let window_end = as_u64(self.window.len());
            for node in &mut nodes {
                if node.locator.raw_value_end >= window_end
                    && matches!(
                        node.kind,
                        JsonValueKind::Number | JsonValueKind::Boolean | JsonValueKind::Null
                    )
                {
                    node.complete = false;
                }
            }
        }

        let limit_reached = match outcome.stop {
            Some(Stop::Limit(field)) => Some(field),
            Some(Stop::Malformed(_)) | None => {
                if outcome.window_truncated {
                    Some(ContextAnalysisLimitField::AnalyzedBytes)
                } else {
                    None
                }
            }
        };
        let status =
            if matches!(outcome.stop, Some(Stop::Malformed(_))) && !outcome.window_truncated {
                ContextAnalysisStatus::Malformed
            } else if limit_reached.is_some() {
                ContextAnalysisStatus::ResourceLimit
            } else if self.duplicate_key_detected {
                ContextAnalysisStatus::Partial
            } else {
                ContextAnalysisStatus::Complete
            };
        RawSpanIndex {
            nodes,
            status,
            limit_reached,
            analyzed_bytes,
            skipped_bytes: outcome.request_bytes.saturating_sub(analyzed_bytes),
            duplicate_key_detected: self.duplicate_key_detected,
        }
    }
}

/// One decoded unit of a JSON string.
///
/// A surrogate that JSON spelled without its pair decodes to no character, so it is kept as its
/// own unit: name comparison stays exact and the public decoder can refuse it instead of
/// inventing a replacement character.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StringUnit {
    Character(char),
    UnpairedSurrogate(u16),
}

impl StringUnit {
    const fn encoded_len(self) -> usize {
        match self {
            Self::Character(character) => character.len_utf8(),
            Self::UnpairedSurrogate(_) => SURROGATE_ENCODED_BYTES,
        }
    }

    fn tagged_code(self) -> u64 {
        match self {
            Self::Character(character) => u64::from(u32::from(character)),
            Self::UnpairedSurrogate(unit) => 0x1_0000_0000_u64.saturating_add(u64::from(unit)),
        }
    }
}

/// Why one JSON string could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StringFault {
    InvalidEscape,
    ControlCharacter,
}

/// Decodes the characters of one JSON string body, escapes included.
struct JsonStringUnits<'text> {
    characters: Chars<'text>,
}

impl<'text> JsonStringUnits<'text> {
    fn new(text: &'text str) -> Self {
        Self {
            characters: text.chars(),
        }
    }

    fn escape(&mut self) -> Result<StringUnit, StringFault> {
        let Some(marker) = self.characters.next() else {
            return Err(StringFault::InvalidEscape);
        };
        let unescaped = match marker {
            '"' => '"',
            '\\' => '\\',
            '/' => '/',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'u' => return self.unicode_escape(),
            _ => return Err(StringFault::InvalidEscape),
        };
        Ok(StringUnit::Character(unescaped))
    }

    fn unicode_escape(&mut self) -> Result<StringUnit, StringFault> {
        let Some(first) = read_hex4(&mut self.characters) else {
            return Err(StringFault::InvalidEscape);
        };
        if (HIGH_SURROGATE_START..LOW_SURROGATE_START).contains(&first) {
            let mut lookahead = self.characters.clone();
            if lookahead.next() == Some('\\') && lookahead.next() == Some('u') {
                if let Some(second) = read_hex4(&mut lookahead) {
                    if (LOW_SURROGATE_START..SURROGATE_END).contains(&second) {
                        if let Some(combined) = combine_surrogates(first, second) {
                            self.characters = lookahead;
                            return Ok(StringUnit::Character(combined));
                        }
                    }
                }
            }
            return Ok(StringUnit::UnpairedSurrogate(first));
        }
        if (LOW_SURROGATE_START..SURROGATE_END).contains(&first) {
            return Ok(StringUnit::UnpairedSurrogate(first));
        }
        char::from_u32(u32::from(first))
            .map(StringUnit::Character)
            .ok_or(StringFault::InvalidEscape)
    }
}

impl Iterator for JsonStringUnits<'_> {
    type Item = Result<StringUnit, StringFault>;

    fn next(&mut self) -> Option<Self::Item> {
        let character = self.characters.next()?;
        if character == '\\' {
            return Some(self.escape());
        }
        if u32::from(character) < 0x20 {
            return Some(Err(StringFault::ControlCharacter));
        }
        Some(Ok(StringUnit::Character(character)))
    }
}

fn read_hex4(characters: &mut Chars<'_>) -> Option<u16> {
    let mut value = 0_u16;
    for _ in 0_u8..4 {
        let digit = u16::try_from(characters.next()?.to_digit(16)?).ok()?;
        value = value.checked_mul(16)?.checked_add(digit)?;
    }
    Some(value)
}

fn combine_surrogates(high: u16, low: u16) -> Option<char> {
    let high = u32::from(high.checked_sub(HIGH_SURROGATE_START)?);
    let low = u32::from(low.checked_sub(LOW_SURROGATE_START)?);
    let code = 0x1_0000_u32.checked_add(high.checked_shl(10)?.checked_add(low)?)?;
    char::from_u32(code)
}

fn mix(hash: u64, value: u64) -> u64 {
    let mut mixed = hash;
    for byte in value.to_le_bytes() {
        mixed = (mixed ^ u64::from(byte)).wrapping_mul(NAME_HASH_PRIME);
    }
    mixed
}

/// Returns the span between the quotes of one quoted span.
const fn inner_span(span: RawSpan) -> Option<RawSpan> {
    if span.len_bytes() < 2 {
        return None;
    }
    RawSpan::new(span.start.saturating_add(1), span.end.saturating_sub(1))
}

/// Borrows the body of one quoted JSON string span as text.
fn quoted_text(request: &[u8], span: RawSpan) -> Option<&str> {
    let bytes = span.slice(request)?;
    let (first, rest) = bytes.split_first()?;
    let (last, inner) = rest.split_last()?;
    if *first != b'"' || *last != b'"' {
        return None;
    }
    std::str::from_utf8(inner).ok()
}

/// Appends the decimal spelling of one array index without allocating.
fn push_decimal(target: &mut String, value: u32) {
    let mut digits = [0_u8; 10];
    let mut written = 0_usize;
    let mut remaining = value;
    loop {
        let digit = u8::try_from(remaining.checked_rem(10).unwrap_or(0)).unwrap_or(0);
        if let Some(slot) = digits.get_mut(written) {
            *slot = b'0'.saturating_add(digit);
        }
        written = written.saturating_add(1);
        remaining = remaining.checked_div(10).unwrap_or(0);
        if remaining == 0 {
            break;
        }
    }
    for offset in (0..written).rev() {
        if let Some(byte) = digits.get(offset) {
            target.push(char::from(*byte));
        }
    }
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
