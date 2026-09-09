//! Deterministic structural detection of the shape of one block's content.
//!
//! A detector answers one narrow question about bytes Tracepress already observed: which
//! well-known shape do they structurally resemble. It parses, counts, and compares against fixed
//! rules. No model, no training corpus, and no similarity search participates, and nothing it
//! produces leaves as more than a [`DetectionResult`] of a kind, an ordinal confidence, and the
//! version of the rules that produced them.
//!
//! Three properties make that answer usable as evidence rather than as decoration.
//!
//! Abstention is a first-class answer. [`DetectionConfidence::Low`] is produced only together
//! with [`DetectedContentKind::Unknown`], so a committed kind always carries at least
//! [`DetectionConfidence::Medium`] and a shape the rules cannot establish is reported as unknown
//! instead of as the most plausible guess.
//!
//! Detection is bounded. At most the configured `max_string_bytes_inspected` prefix of the content
//! is examined, JSON nesting is descended a fixed number of levels, and a verdict reached from a
//! prefix rather than from the whole content never claims [`DetectionConfidence::High`].
//!
//! Detection is total. Invalid UTF-8, an embedded NUL, and arbitrary binary bytes are a kind of
//! their own rather than an error path; no input reaches a panic, an allocation sized by a
//! declared length, or unbounded recursion.

use std::num::NonZeroUsize;

use crate::{
    ContextAnalysisLimits, ContextBlockKind, DetectedContentKind, DetectionConfidence,
    DetectionResult,
};

/// Version of the structural detector rules recorded with every [`DetectionResult`].
pub const STRUCTURAL_DETECTOR_VERSION: u32 = 1;

/// Nesting levels a detector descends while establishing a JSON shape.
///
/// The bound is the detector's own ceiling on recursion depth, applied on top of the configured
/// `max_json_depth`: content nested deeper is reported as unknown rather than being descended.
const MAX_JSON_DEPTH_INSPECTED: u64 = 64;
/// Share of inspected bytes above which non-textual control bytes make content binary-like.
const CONTROL_BYTE_PERCENT: u64 = 5;
/// Share of lines carrying code punctuation above which content is punctuation-dense.
const CODE_DENSE_PERCENT: u64 = 50;
/// Share of non-space characters that must be letters before content reads as prose.
const PROSE_LETTER_PERCENT: u64 = 60;
/// Words per line, in hundredths, below which content is too sparse to read as prose.
const PROSE_WORDS_PER_LINE_PERCENT: u64 = 300;
/// Share of lines that must parse as one JSON value before content reads as newline-delimited.
const NDJSON_PARSED_PERCENT: u64 = 80;
/// Share of lines that must carry a timestamp before content reads unambiguously as a log.
const LOG_TIMESTAMP_PERCENT: u64 = 80;
/// Share of lines that must carry a timestamp or a level before content reads as a log.
const LOG_WEAK_PERCENT: u64 = 50;
/// Share of lines that must name a path and line before content reads unambiguously as a search.
const SEARCH_STRONG_PERCENT: u64 = 80;
/// Share of lines that must name a path and line before content reads as a search.
const SEARCH_WEAK_PERCENT: u64 = 50;
/// Share of lines that must carry a test-runner marker before content reads as test output.
const TEST_MARKER_PERCENT: u64 = 30;
/// Largest path length a search-result line may name.
const MAX_PATH_BYTES: usize = 512;
/// Largest file extension length a plausible path may carry.
const MAX_EXTENSION_BYTES: usize = 8;

/// Bounded structural metadata of the block whose content is being detected.
///
/// The kind decides whether the content is inspectable at all, and the declared length lets a
/// detector that received only a prefix cap its own confidence. Neither carries content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockContentMetadata {
    /// Canonical kind of the block the content belongs to.
    pub kind: ContextBlockKind,
    /// Complete length of the block's content in bytes, when the caller knows it.
    ///
    /// A value larger than the content handed to the detector means the caller already truncated,
    /// so the verdict describes a prefix. It is compared and never used to size an allocation.
    pub content_bytes: Option<u64>,
}

impl BlockContentMetadata {
    /// Describes a block of `kind` whose content length is not separately known.
    #[must_use]
    pub const fn new(kind: ContextBlockKind) -> Self {
        Self {
            kind,
            content_bytes: None,
        }
    }

    /// Declares the complete content length of the block.
    #[must_use]
    pub const fn with_content_bytes(self, content_bytes: u64) -> Self {
        Self {
            kind: self.kind,
            content_bytes: Some(content_bytes),
        }
    }

    /// Returns whether this kind of block carries content a detector may read.
    ///
    /// A reference is never resolved and an opaque item is never decoded by this phase, so their
    /// envelopes are not classified as if their bytes were the referenced content.
    const fn content_is_inspectable(self) -> bool {
        !matches!(
            self.kind,
            ContextBlockKind::ImageReference
                | ContextBlockKind::FileReference
                | ContextBlockKind::ItemReference
                | ContextBlockKind::PromptReference
                | ContextBlockKind::ProviderStateReference
                | ContextBlockKind::Opaque
                | ContextBlockKind::OpaqueReasoning
        )
    }

    /// Returns whether the caller declared more content than it handed over.
    fn declares_more_than(self, observed: usize) -> bool {
        self.content_bytes
            .is_some_and(|total| total > u64::try_from(observed).unwrap_or(u64::MAX))
    }
}

/// A deterministic structural detector of one content shape.
pub trait ShadowContentDetector {
    /// Returns the version of the rules this detector applies.
    #[must_use]
    fn detector_version(&self) -> u32;

    /// Detects the structural shape of `content` observed inside the described block.
    ///
    /// The content is borrowed and never modified. Any input is answered, including invalid
    /// UTF-8, embedded NUL bytes, and empty content.
    #[must_use]
    fn detect(&self, content: &[u8], metadata: BlockContentMetadata) -> DetectionResult;
}

/// The structural detector this analysis version ships.
///
/// It recognises a whole JSON document, newline-delimited JSON, unified diffs, test-runner
/// output, `path:line` search results, timestamped or levelled logs, source code, prose, and
/// binary-like bytes, and abstains for everything else.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StructuralContentDetector {
    inspected_bytes_budget: NonZeroUsize,
    max_json_depth: u64,
}

impl StructuralContentDetector {
    /// Creates a detector bounded by the configured analysis limits.
    ///
    /// The inspection budget is `max_string_bytes_inspected`, and the descended nesting depth is
    /// the smaller of the configured `max_json_depth` and the detector's own ceiling.
    #[must_use]
    pub const fn new(limits: &ContextAnalysisLimits) -> Self {
        let configured_depth = limits.max_json_depth.get();
        Self {
            inspected_bytes_budget: limits.max_string_bytes_inspected,
            max_json_depth: if configured_depth < MAX_JSON_DEPTH_INSPECTED {
                configured_depth
            } else {
                MAX_JSON_DEPTH_INSPECTED
            },
        }
    }

    /// Returns the content prefix length this detector inspects, in bytes.
    #[must_use]
    pub const fn inspected_bytes_budget(&self) -> usize {
        self.inspected_bytes_budget.get()
    }

    fn verdict(&self, content: &[u8], metadata: BlockContentMetadata) -> Verdict {
        if !metadata.content_is_inspectable() {
            return Verdict::ABSTAINED;
        }
        let inspected = content
            .get(..self.inspected_bytes_budget.get())
            .unwrap_or(content);
        if inspected.is_empty() {
            return Verdict::ABSTAINED;
        }
        let bounded = inspected.len() < content.len() || metadata.declares_more_than(content.len());
        if let Some(binary) = binary_verdict(inspected) {
            return binary;
        }
        let Some(text) = decode_prefix(inspected, bounded) else {
            return Verdict::committed(DetectedContentKind::BinaryLike, DetectionConfidence::High);
        };
        let verdict = classify_text(text, bounded, self.max_json_depth);
        if bounded { verdict.bounded() } else { verdict }
    }
}

impl ShadowContentDetector for StructuralContentDetector {
    fn detector_version(&self) -> u32 {
        STRUCTURAL_DETECTOR_VERSION
    }

    fn detect(&self, content: &[u8], metadata: BlockContentMetadata) -> DetectionResult {
        let verdict = self.verdict(content, metadata);
        DetectionResult {
            kind: verdict.kind,
            confidence: verdict.confidence,
            detector_version: STRUCTURAL_DETECTOR_VERSION,
        }
    }
}

/// One detector's committed kind and confidence, before it is versioned.
#[derive(Clone, Copy, Eq, PartialEq)]
struct Verdict {
    kind: DetectedContentKind,
    confidence: DetectionConfidence,
}

impl Verdict {
    /// The only verdict that carries [`DetectionConfidence::Low`].
    const ABSTAINED: Self = Self {
        kind: DetectedContentKind::Unknown,
        confidence: DetectionConfidence::Low,
    };

    const fn committed(kind: DetectedContentKind, confidence: DetectionConfidence) -> Self {
        Self { kind, confidence }
    }

    /// Caps a verdict reached from a prefix of the content.
    const fn bounded(self) -> Self {
        match self.confidence {
            DetectionConfidence::High => Self {
                kind: self.kind,
                confidence: DetectionConfidence::Medium,
            },
            _ => self,
        }
    }
}

/// Recognises bytes that do not decode as text at all.
fn binary_verdict(inspected: &[u8]) -> Option<Verdict> {
    if inspected.contains(&0) {
        return Some(Verdict::committed(
            DetectedContentKind::BinaryLike,
            DetectionConfidence::High,
        ));
    }
    let controls = inspected
        .iter()
        .filter(|byte| is_binary_control(**byte))
        .count();
    ratio_at_least(controls, inspected.len(), CONTROL_BYTE_PERCENT)
        .then(|| Verdict::committed(DetectedContentKind::BinaryLike, DetectionConfidence::Medium))
}

/// Returns whether a byte is a control character no text format uses for layout.
///
/// Tab, newline, carriage return, and escape are excluded: the first three are layout, and an
/// escape sequence appears in ordinary captured terminal output.
const fn is_binary_control(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1a | 0x1c..=0x1f | 0x7f)
}

/// Decodes the inspected prefix, tolerating a character the prefix cut in half.
fn decode_prefix(inspected: &[u8], bounded: bool) -> Option<&str> {
    match std::str::from_utf8(inspected) {
        Ok(text) => Some(text),
        Err(error) if bounded && error.error_len().is_none() => inspected
            .get(..error.valid_up_to())
            .and_then(|head| std::str::from_utf8(head).ok()),
        Err(_) => None,
    }
}

/// Applies every structural rule to decoded content, in a fixed precedence.
fn classify_text(text: &str, bounded: bool, max_json_depth: u64) -> Verdict {
    match scan_json_document(text.as_bytes(), max_json_depth) {
        JsonDocument::Complete => {
            return Verdict::committed(DetectedContentKind::Json, DetectionConfidence::High);
        }
        JsonDocument::Incomplete if bounded => {
            return Verdict::committed(DetectedContentKind::Json, DetectionConfidence::Medium);
        }
        JsonDocument::DepthExceeded => return Verdict::ABSTAINED,
        JsonDocument::Incomplete | JsonDocument::Invalid => {}
    }
    let signals = LineSignals::of(text, max_json_depth);
    ndjson_verdict(&signals)
        .or_else(|| diff_verdict(&signals))
        .or_else(|| test_verdict(&signals))
        .or_else(|| search_verdict(&signals))
        .or_else(|| log_verdict(&signals))
        .or_else(|| source_verdict(&signals))
        .or_else(|| prose_verdict(&signals))
        .unwrap_or(Verdict::ABSTAINED)
}

fn ndjson_verdict(signals: &LineSignals) -> Option<Verdict> {
    if signals.considered < 2 || signals.json_containers < 2 {
        return None;
    }
    if signals.json_values == signals.considered && signals.considered >= 3 {
        return Some(Verdict::committed(
            DetectedContentKind::Ndjson,
            DetectionConfidence::High,
        ));
    }
    ratio_at_least(
        signals.json_values,
        signals.considered,
        NDJSON_PARSED_PERCENT,
    )
    .then(|| Verdict::committed(DetectedContentKind::Ndjson, DetectionConfidence::Medium))
}

fn diff_verdict(signals: &LineSignals) -> Option<Verdict> {
    let file_pair = signals.diff_old_files >= 1 && signals.diff_new_files >= 1;
    if signals.diff_hunks >= 1 && (file_pair || signals.diff_headers >= 1) {
        return Some(Verdict::committed(
            DetectedContentKind::Diff,
            DetectionConfidence::High,
        ));
    }
    ((signals.diff_hunks >= 1 || file_pair) && signals.diff_changes >= 2)
        .then(|| Verdict::committed(DetectedContentKind::Diff, DetectionConfidence::Medium))
}

fn test_verdict(signals: &LineSignals) -> Option<Verdict> {
    if signals.considered < 2 || signals.test_markers == 0 {
        return None;
    }
    let dense = ratio_at_least(
        signals.test_markers,
        signals.considered,
        TEST_MARKER_PERCENT,
    );
    if signals.test_summaries >= 1 && dense {
        return Some(Verdict::committed(
            DetectedContentKind::TestResults,
            DetectionConfidence::High,
        ));
    }
    (dense && (signals.test_summaries >= 1 || signals.test_markers >= 2)).then(|| {
        Verdict::committed(
            DetectedContentKind::TestResults,
            DetectionConfidence::Medium,
        )
    })
}

fn search_verdict(signals: &LineSignals) -> Option<Verdict> {
    if signals.considered >= 3
        && ratio_at_least(
            signals.path_line_prefixes,
            signals.considered,
            SEARCH_STRONG_PERCENT,
        )
    {
        return Some(Verdict::committed(
            DetectedContentKind::SearchResults,
            DetectionConfidence::High,
        ));
    }
    (signals.considered >= 2
        && ratio_at_least(
            signals.path_line_prefixes,
            signals.considered,
            SEARCH_WEAK_PERCENT,
        ))
    .then(|| {
        Verdict::committed(
            DetectedContentKind::SearchResults,
            DetectionConfidence::Medium,
        )
    })
}

fn log_verdict(signals: &LineSignals) -> Option<Verdict> {
    let timestamped = ratio_at_least(
        signals.timestamps,
        signals.considered,
        LOG_TIMESTAMP_PERCENT,
    );
    if signals.considered >= 3 && timestamped {
        return Some(Verdict::committed(
            DetectedContentKind::Log,
            DetectionConfidence::High,
        ));
    }
    let weakly_timestamped =
        ratio_at_least(signals.timestamps, signals.considered, LOG_WEAK_PERCENT);
    let levelled = signals.considered >= 3
        && ratio_at_least(signals.levels, signals.considered, LOG_WEAK_PERCENT);
    (signals.considered >= 2 && (weakly_timestamped || levelled))
        .then(|| Verdict::committed(DetectedContentKind::Log, DetectionConfidence::Medium))
}

fn source_verdict(signals: &LineSignals) -> Option<Verdict> {
    let categories = [
        signals.code_declarations,
        signals.code_imports,
        signals.code_comments,
    ]
    .iter()
    .filter(|observed| **observed > 0)
    .count();
    let dense = ratio_at_least(
        signals.code_punctuation,
        signals.considered,
        CODE_DENSE_PERCENT,
    );
    if categories >= 2 && dense && signals.considered >= 3 {
        return Some(Verdict::committed(
            DetectedContentKind::SourceCode,
            DetectionConfidence::High,
        ));
    }
    let committed = (categories >= 2 && signals.considered >= 2)
        || (categories >= 1 && dense && signals.considered >= 3);
    committed
        .then(|| Verdict::committed(DetectedContentKind::SourceCode, DetectionConfidence::Medium))
}

/// Recognises prose, which is the absence of structure rather than a structure of its own.
///
/// It is therefore never committed above [`DetectionConfidence::Medium`].
fn prose_verdict(signals: &LineSignals) -> Option<Verdict> {
    let non_space = signals
        .letters
        .saturating_add(signals.digits)
        .saturating_add(signals.symbols);
    let prose = signals.considered >= 1
        && ratio_at_least(signals.letters, non_space, PROSE_LETTER_PERCENT)
        && ratio_at_least(
            signals.words,
            signals.considered,
            PROSE_WORDS_PER_LINE_PERCENT,
        )
        && !ratio_at_least(
            signals.code_punctuation,
            signals.considered,
            CODE_DENSE_PERCENT,
        );
    prose.then(|| Verdict::committed(DetectedContentKind::PlainText, DetectionConfidence::Medium))
}

/// Returns whether `part` is at least `percent` of `whole`, with an empty whole never qualifying.
fn ratio_at_least(part: usize, whole: usize, percent: u64) -> bool {
    if whole == 0 {
        return false;
    }
    let part = u64::try_from(part).unwrap_or(u64::MAX);
    let whole = u64::try_from(whole).unwrap_or(u64::MAX);
    part.saturating_mul(100) >= percent.saturating_mul(whole)
}

/// Counted structural signals of the inspected content.
///
/// Every counter is a count of lines or characters. Nothing here retains content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LineSignals {
    considered: usize,
    json_values: usize,
    json_containers: usize,
    timestamps: usize,
    levels: usize,
    path_line_prefixes: usize,
    test_markers: usize,
    test_summaries: usize,
    diff_headers: usize,
    diff_old_files: usize,
    diff_new_files: usize,
    diff_hunks: usize,
    diff_changes: usize,
    code_declarations: usize,
    code_imports: usize,
    code_comments: usize,
    code_punctuation: usize,
    letters: usize,
    digits: usize,
    symbols: usize,
    words: usize,
}

impl LineSignals {
    fn of(text: &str, max_json_depth: u64) -> Self {
        let mut signals = Self {
            words: text.split_whitespace().count(),
            ..Self::default()
        };
        for character in text.chars() {
            if character.is_alphabetic() {
                signals.letters = signals.letters.saturating_add(1);
            } else if character.is_numeric() {
                signals.digits = signals.digits.saturating_add(1);
            } else if !character.is_whitespace() {
                signals.symbols = signals.symbols.saturating_add(1);
            }
        }
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            signals.considered = signals.considered.saturating_add(1);
            signals.observe_json(trimmed, max_json_depth);
            signals.observe_diff(trimmed);
            signals.observe_log(trimmed);
            signals.observe_test(trimmed);
            signals.observe_code(trimmed);
            if looks_like_path_line(trimmed) {
                signals.path_line_prefixes = signals.path_line_prefixes.saturating_add(1);
            }
        }
        signals
    }

    fn observe_json(&mut self, trimmed: &str, max_json_depth: u64) {
        if !matches!(
            scan_json_document(trimmed.as_bytes(), max_json_depth),
            JsonDocument::Complete
        ) {
            return;
        }
        self.json_values = self.json_values.saturating_add(1);
        if matches!(trimmed.as_bytes().first(), Some(b'{' | b'[')) {
            self.json_containers = self.json_containers.saturating_add(1);
        }
    }

    fn observe_diff(&mut self, trimmed: &str) {
        if trimmed.starts_with("diff --git ") || trimmed.starts_with("Index: ") {
            self.diff_headers = self.diff_headers.saturating_add(1);
        } else if trimmed.starts_with("--- ") {
            self.diff_old_files = self.diff_old_files.saturating_add(1);
        } else if trimmed.starts_with("+++ ") {
            self.diff_new_files = self.diff_new_files.saturating_add(1);
        } else if trimmed.starts_with('+') || trimmed.starts_with('-') {
            self.diff_changes = self.diff_changes.saturating_add(1);
        }
        if trimmed.starts_with("@@")
            && trimmed
                .get(2..)
                .is_some_and(|rest| rest.contains("@@") && rest.contains('-'))
        {
            self.diff_hunks = self.diff_hunks.saturating_add(1);
        }
    }

    fn observe_log(&mut self, trimmed: &str) {
        if looks_like_timestamp(trimmed) {
            self.timestamps = self.timestamps.saturating_add(1);
        }
        if carries_log_level(trimmed) {
            self.levels = self.levels.saturating_add(1);
        }
    }

    fn observe_test(&mut self, trimmed: &str) {
        if is_test_marker(trimmed) {
            self.test_markers = self.test_markers.saturating_add(1);
        }
        if is_test_summary(trimmed) {
            self.test_summaries = self.test_summaries.saturating_add(1);
        }
    }

    fn observe_code(&mut self, trimmed: &str) {
        if starts_with_any(trimmed, &DECLARATION_KEYWORDS) {
            self.code_declarations = self.code_declarations.saturating_add(1);
        }
        if starts_with_any(trimmed, &IMPORT_KEYWORDS) {
            self.code_imports = self.code_imports.saturating_add(1);
        }
        if starts_with_any(trimmed, &COMMENT_MARKERS) {
            self.code_comments = self.code_comments.saturating_add(1);
        }
        if trimmed.ends_with(':')
            || trimmed.contains(|character: char| CODE_PUNCTUATION.contains(&character))
        {
            self.code_punctuation = self.code_punctuation.saturating_add(1);
        }
    }
}

/// Declaration keywords a source line may open with, across the common languages.
const DECLARATION_KEYWORDS: [&str; 19] = [
    "fn ",
    "pub ",
    "def ",
    "class ",
    "func ",
    "function ",
    "impl ",
    "struct ",
    "enum ",
    "trait ",
    "interface ",
    "public ",
    "private ",
    "protected ",
    "const ",
    "let ",
    "var ",
    "type ",
    "namespace ",
];
/// Import keywords a source line may open with.
const IMPORT_KEYWORDS: [&str; 7] = [
    "import ", "from ", "use ", "using ", "#include", "package ", "require(",
];
/// Comment markers a source line may open with.
const COMMENT_MARKERS: [&str; 4] = ["//", "/*", "#!", "<!--"];
/// Punctuation whose presence marks a line as code rather than prose.
const CODE_PUNCTUATION: [char; 9] = ['{', '}', ';', '(', ')', '=', '[', ']', '>'];
/// Level tokens a log line may carry, matched case-sensitively so prose does not qualify.
const LOG_LEVELS: [&str; 10] = [
    "TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "FATAL", "CRITICAL", "NOTICE", "SEVERE",
];
/// Diagnostic prefixes a compiler log line may open with.
const DIAGNOSTIC_PREFIXES: [&str; 3] = ["error", "warning", "note"];

fn starts_with_any(trimmed: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| trimmed.starts_with(prefix))
}

/// Returns whether a line opens with a timestamp.
///
/// Only an anchored timestamp counts, so a path or an identifier that happens to contain digits
/// and colons further along the line cannot make arbitrary content look like a log.
fn looks_like_timestamp(trimmed: &str) -> bool {
    let bytes = match trimmed.as_bytes() {
        [b'[' | b'(', rest @ ..] => rest,
        bytes => bytes,
    };
    if let Some(after_date) = take_iso_date(bytes) {
        return match after_date {
            [b'T' | b't' | b' ' | b'_', rest @ ..] => take_clock(rest).is_some(),
            _ => false,
        };
    }
    take_clock(bytes).is_some()
}

/// Returns the bytes after a `YYYY-MM-DD` date.
fn take_iso_date(bytes: &[u8]) -> Option<&[u8]> {
    let rest = take_byte(take_digits(bytes, 4)?, b'-')?;
    let rest = take_byte(take_digits(rest, 2)?, b'-')?;
    take_digits(rest, 2)
}

/// Returns the bytes after a `HH:MM:SS` clock.
fn take_clock(bytes: &[u8]) -> Option<&[u8]> {
    let rest = take_byte(take_digits(bytes, 2)?, b':')?;
    let rest = take_byte(take_digits(rest, 2)?, b':')?;
    take_digits(rest, 2)
}

fn take_digits(bytes: &[u8], count: usize) -> Option<&[u8]> {
    let head = bytes.get(..count)?;
    head.iter()
        .all(u8::is_ascii_digit)
        .then(|| bytes.get(count..))
        .flatten()
}

fn take_byte(bytes: &[u8], expected: u8) -> Option<&[u8]> {
    (bytes.first() == Some(&expected))
        .then(|| bytes.get(1..))
        .flatten()
}

/// Returns whether a line carries an uppercase level token or a compiler diagnostic prefix.
fn carries_log_level(trimmed: &str) -> bool {
    if trimmed
        .split(|character: char| !character.is_ascii_alphabetic())
        .any(|token| LOG_LEVELS.contains(&token))
    {
        return true;
    }
    DIAGNOSTIC_PREFIXES.iter().any(|prefix| {
        trimmed
            .strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest.as_bytes().first(), Some(b':' | b'[')))
    })
}

/// Returns whether a line opens with a `path:line` reference, as every search tool prints.
fn looks_like_path_line(trimmed: &str) -> bool {
    let Some((path, rest)) = trimmed.split_once(':') else {
        return false;
    };
    if !is_plausible_path(path) {
        return false;
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    matches!(
        rest.as_bytes().get(digits),
        None | Some(b':' | b' ' | b'\t')
    )
}

/// Returns whether a token can be a file path rather than an arbitrary word.
fn is_plausible_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.starts_with('-')
        || path.contains(char::is_whitespace)
    {
        return false;
    }
    path.contains('/') || has_file_extension(path)
}

/// Returns whether a path ends in a short alphabetic-led extension.
fn has_file_extension(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && !extension.is_empty()
            && extension.len() <= MAX_EXTENSION_BYTES
            && extension.starts_with(|character: char| character.is_ascii_alphabetic())
            && extension.chars().all(char::is_alphanumeric)
    })
}

/// Returns whether a line is one test-runner outcome.
fn is_test_marker(trimmed: &str) -> bool {
    (trimmed.starts_with("test ") && (trimmed.contains(" ... ") || trimmed.ends_with(" ok")))
        || starts_with_any(
            trimmed,
            &[
                "--- PASS",
                "--- FAIL",
                "--- SKIP",
                "PASS ",
                "FAIL ",
                "ok ",
                "not ok ",
                "[ RUN      ]",
                "[       OK ]",
                "[  FAILED  ]",
                "\u{2713}",
                "\u{2717}",
            ],
        )
        || trimmed.contains(" PASSED")
        || trimmed.contains(" FAILED")
}

/// Returns whether a line is one test-runner summary.
fn is_test_summary(trimmed: &str) -> bool {
    starts_with_any(
        trimmed,
        &["test result:", "Tests run:", "OK (", "FAILED (", "Ran "],
    ) || (trimmed.starts_with("running ") && trimmed.ends_with(" tests"))
        || (trimmed.contains(" passed")
            && (trimmed.contains(" failed")
                || trimmed.contains(" skipped")
                || trimmed.contains(" ignored")
                || trimmed.contains(" total")))
}

/// Outcome of scanning bytes that should hold exactly one JSON value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonDocument {
    /// The bytes are one JSON value followed by whitespace only.
    Complete,
    /// The bytes end inside a value that is valid so far.
    Incomplete,
    /// The bytes violate JSON syntax or carry trailing content.
    Invalid,
    /// Nesting passed the inspected depth before a verdict was reached.
    DepthExceeded,
}

/// Scans bytes that should hold exactly one JSON value, bounded by an inspected nesting depth.
fn scan_json_document(bytes: &[u8], max_json_depth: u64) -> JsonDocument {
    JsonScanner {
        bytes,
        max_json_depth,
    }
    .document()
}

/// A bounded structural scan of one JSON value.
///
/// The scan validates structure without materialising a value, so nothing it reads is retained
/// and nothing it reads is allocated. Recursion descends at most `max_json_depth` levels.
struct JsonScanner<'bytes> {
    bytes: &'bytes [u8],
    max_json_depth: u64,
}

impl JsonScanner<'_> {
    /// Scans bytes that should hold exactly one JSON value.
    fn document(&self) -> JsonDocument {
        let start = skip_whitespace(self.bytes, 0);
        if self.bytes.get(start).is_none() {
            return JsonDocument::Invalid;
        }
        match self.value(start, 1) {
            ValueScan::Complete(end) => {
                if self.bytes.get(skip_whitespace(self.bytes, end)).is_none() {
                    JsonDocument::Complete
                } else {
                    JsonDocument::Invalid
                }
            }
            ValueScan::Incomplete => JsonDocument::Incomplete,
            ValueScan::Invalid => JsonDocument::Invalid,
            ValueScan::DepthExceeded => JsonDocument::DepthExceeded,
        }
    }

    fn value(&self, at: usize, depth: u64) -> ValueScan {
        if depth > self.max_json_depth {
            return ValueScan::DepthExceeded;
        }
        let Some(&byte) = self.bytes.get(at) else {
            return ValueScan::Incomplete;
        };
        match byte {
            b'{' => self.object(at, depth),
            b'[' => self.array(at, depth),
            b'"' => scan_string(self.bytes, at),
            b't' => scan_literal(self.bytes, at, b"true"),
            b'f' => scan_literal(self.bytes, at, b"false"),
            b'n' => scan_literal(self.bytes, at, b"null"),
            b'-' | b'0'..=b'9' => scan_number(self.bytes, at),
            _ => ValueScan::Invalid,
        }
    }

    fn object(&self, at: usize, depth: u64) -> ValueScan {
        let bytes = self.bytes;
        let mut index = skip_whitespace(bytes, at.saturating_add(1));
        match bytes.get(index) {
            None => return ValueScan::Incomplete,
            Some(b'}') => return ValueScan::Complete(index.saturating_add(1)),
            Some(_) => {}
        }
        loop {
            match bytes.get(index) {
                None => return ValueScan::Incomplete,
                Some(b'"') => {}
                Some(_) => return ValueScan::Invalid,
            }
            match scan_string(bytes, index) {
                ValueScan::Complete(end) => index = skip_whitespace(bytes, end),
                other => return other,
            }
            match bytes.get(index) {
                None => return ValueScan::Incomplete,
                Some(b':') => index = skip_whitespace(bytes, index.saturating_add(1)),
                Some(_) => return ValueScan::Invalid,
            }
            match self.value(index, depth.saturating_add(1)) {
                ValueScan::Complete(end) => index = skip_whitespace(bytes, end),
                other => return other,
            }
            match bytes.get(index) {
                None => return ValueScan::Incomplete,
                Some(b',') => index = skip_whitespace(bytes, index.saturating_add(1)),
                Some(b'}') => return ValueScan::Complete(index.saturating_add(1)),
                Some(_) => return ValueScan::Invalid,
            }
        }
    }

    fn array(&self, at: usize, depth: u64) -> ValueScan {
        let bytes = self.bytes;
        let mut index = skip_whitespace(bytes, at.saturating_add(1));
        match bytes.get(index) {
            None => return ValueScan::Incomplete,
            Some(b']') => return ValueScan::Complete(index.saturating_add(1)),
            Some(_) => {}
        }
        loop {
            match self.value(index, depth.saturating_add(1)) {
                ValueScan::Complete(end) => index = skip_whitespace(bytes, end),
                other => return other,
            }
            match bytes.get(index) {
                None => return ValueScan::Incomplete,
                Some(b',') => index = skip_whitespace(bytes, index.saturating_add(1)),
                Some(b']') => return ValueScan::Complete(index.saturating_add(1)),
                Some(_) => return ValueScan::Invalid,
            }
        }
    }
}

/// Outcome of scanning one JSON value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValueScan {
    /// One complete value ending before this offset.
    Complete(usize),
    /// The bytes end inside a value that is valid so far.
    Incomplete,
    /// The bytes violate JSON syntax.
    Invalid,
    /// Nesting passed the inspected depth.
    DepthExceeded,
}

fn scan_string(bytes: &[u8], at: usize) -> ValueScan {
    let mut index = at.saturating_add(1);
    loop {
        let Some(&byte) = bytes.get(index) else {
            return ValueScan::Incomplete;
        };
        match byte {
            b'"' => return ValueScan::Complete(index.saturating_add(1)),
            b'\\' => match scan_escape(bytes, index) {
                Ok(next) => index = next,
                Err(scan) => return scan,
            },
            0x00..=0x1f => return ValueScan::Invalid,
            _ => index = index.saturating_add(1),
        }
    }
}

/// Returns the offset after one string escape, or the scan that ends the string.
fn scan_escape(bytes: &[u8], at: usize) -> Result<usize, ValueScan> {
    let Some(&escape) = bytes.get(at.saturating_add(1)) else {
        return Err(ValueScan::Incomplete);
    };
    match escape {
        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => Ok(at.saturating_add(2)),
        b'u' => {
            let start = at.saturating_add(2);
            let end = start.saturating_add(4);
            match bytes.get(start..end) {
                None => Err(ValueScan::Incomplete),
                Some(hex) if hex.iter().all(u8::is_ascii_hexdigit) => Ok(end),
                Some(_) => Err(ValueScan::Invalid),
            }
        }
        _ => Err(ValueScan::Invalid),
    }
}

fn scan_literal(bytes: &[u8], at: usize, literal: &[u8]) -> ValueScan {
    let end = at.saturating_add(literal.len());
    match bytes.get(at..end) {
        Some(found) if found == literal => ValueScan::Complete(end),
        Some(_) => ValueScan::Invalid,
        None => match bytes.get(at..) {
            Some(tail) if literal.starts_with(tail) => ValueScan::Incomplete,
            _ => ValueScan::Invalid,
        },
    }
}

fn scan_number(bytes: &[u8], at: usize) -> ValueScan {
    let mut index = at;
    if bytes.get(index) == Some(&b'-') {
        index = index.saturating_add(1);
    }
    index = match scan_digits(bytes, index) {
        Ok(end) => end,
        Err(scan) => return scan,
    };
    if bytes.get(index) == Some(&b'.') {
        index = match scan_digits(bytes, index.saturating_add(1)) {
            Ok(end) => end,
            Err(scan) => return scan,
        };
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        let mut exponent = index.saturating_add(1);
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent = exponent.saturating_add(1);
        }
        index = match scan_digits(bytes, exponent) {
            Ok(end) => end,
            Err(scan) => return scan,
        };
    }
    ValueScan::Complete(index)
}

/// Returns the offset after at least one digit, or the scan that ends the number.
fn scan_digits(bytes: &[u8], from: usize) -> Result<usize, ValueScan> {
    let mut index = from;
    while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
        index = index.saturating_add(1);
    }
    if index > from {
        Ok(index)
    } else if bytes.get(index).is_none() {
        Err(ValueScan::Incomplete)
    } else {
        Err(ValueScan::Invalid)
    }
}

fn skip_whitespace(bytes: &[u8], from: usize) -> usize {
    let mut index = from;
    while matches!(bytes.get(index), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        index = index.saturating_add(1);
    }
    index
}
