//! Local token estimation for observed content bytes.
//!
//! A local estimate is never provider truth. Every value this module produces carries the identity
//! and version of the estimator that produced it and an [`EstimateConfidence`] that says how it was
//! produced; nothing here can be read as an exact count, and nothing here is ever summed with
//! provider-observed usage.
//!
//! Phase 3 ships exactly one estimator: [`StructuralHeuristicEstimator`], a deterministic
//! structural heuristic reported as [`EstimateConfidence::Heuristic`].
//! [`EstimateConfidence::ModelMapped`] and [`EstimateConfidence::GenericTokenizer`] stay
//! unimplemented rather than approximated by a heuristic wearing their name: a real tokenizer
//! arrives with its own vetted dependency and mapping table, and until then coverage reports
//! honestly that no model-mapped estimate exists.

use core::fmt;
use core::str;

use crate::{BoundedMetadataText, ContextAnalysisLimits, EstimateConfidence, TokenEstimate};

/// One estimation request: the model that was named, the content bytes, and the bounds.
///
/// The bytes are borrowed and never mutated. Its [`Debug`] representation reports the model
/// presence and the content length, never a byte of the content or a string an untrusted request
/// chose.
#[allow(
    clippy::exhaustive_structs,
    reason = "the contract fixes the estimator input; a new input must break every estimator"
)]
#[derive(Clone, Copy)]
pub struct EstimationRequest<'content> {
    /// Model the observed request named, absent when it is unknown.
    pub model: Option<&'content str>,
    /// Content bytes to estimate.
    pub content: &'content [u8],
    /// Bounds this estimation must respect.
    pub limits: ContextAnalysisLimits,
}

impl fmt::Debug for EstimationRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EstimationRequest")
            .field("model_named", &self.model.is_some())
            .field("content_bytes", &self.content.len())
            .field("limits", &self.limits)
            .finish()
    }
}

/// Maps a model name plus content bytes to a locally estimated token count.
///
/// The model name is accepted by every estimator so a future model-mapped tokenizer can replace
/// the implementation without changing a single call site. The estimator shipped in this phase has
/// no model-to-encoding mapping and therefore ignores it, which is exactly why its estimates leave
/// [`TokenEstimate::encoding`] absent rather than naming an encoding it did not use.
///
/// An implementation must be deterministic for identical bytes, must inspect no more than the
/// analysis limits admit, and must report [`TokenEstimation::Unavailable`] rather than zero when it
/// cannot estimate at all.
pub trait TokenEstimator {
    /// Stable identity recorded with every estimate this estimator produces.
    fn identity(&self) -> &BoundedMetadataText;

    /// Version of this estimator's rules.
    ///
    /// A rule change that moves estimates must move this version, because a stored estimate is
    /// only comparable to another estimate of the same estimator and version.
    fn version(&self) -> u32;

    /// Estimates the tokens of the requested content within the requested bounds.
    ///
    /// The content is borrowed and never mutated. Bytes beyond the admitted bound are not
    /// inspected and are reported as skipped instead of being silently folded into the estimate.
    fn estimate(&self, request: &EstimationRequest<'_>) -> TokenEstimation;
}

/// Outcome of one bounded token estimation.
///
/// The absence of an estimate is a distinct outcome from an estimate of zero: content that cannot
/// be estimated reports [`TokenEstimation::Unavailable`], while genuinely empty content reports a
/// known zero.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TokenEstimation {
    /// Every content byte was inspected.
    Complete(TokenEstimate),
    /// A bound stopped the inspection; the work completed inside the bound is kept.
    Bounded {
        /// Estimate over the inspected prefix only.
        estimate: TokenEstimate,
        /// Content bytes actually inspected.
        inspected_bytes: u64,
        /// Content bytes left uninspected by the bound.
        skipped_bytes: u64,
    },
    /// No estimate applies to this content.
    Unavailable(EstimateUnavailable),
}

impl TokenEstimation {
    /// Returns the estimate, absent when none applies.
    #[must_use]
    pub const fn estimate(&self) -> Option<&TokenEstimate> {
        match self {
            Self::Complete(estimate) | Self::Bounded { estimate, .. } => Some(estimate),
            Self::Unavailable(_) => None,
        }
    }

    /// Returns the estimated tokens, absent when no estimate applies.
    ///
    /// An unavailable estimate is [`None`], never `Some(0)`.
    #[must_use]
    pub const fn tokens(&self) -> Option<u64> {
        match self.estimate() {
            Some(estimate) => Some(estimate.tokens),
            None => None,
        }
    }

    /// Returns whether every content byte was inspected.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }

    /// Returns the content bytes a bound left uninspected.
    #[must_use]
    pub const fn skipped_bytes(&self) -> u64 {
        match self {
            Self::Bounded { skipped_bytes, .. } => *skipped_bytes,
            Self::Complete(_) | Self::Unavailable(_) => 0,
        }
    }

    /// Returns why no estimate applies, absent when one does.
    #[must_use]
    pub const fn unavailable_reason(&self) -> Option<EstimateUnavailable> {
        match self {
            Self::Unavailable(reason) => Some(*reason),
            Self::Complete(_) | Self::Bounded { .. } => None,
        }
    }
}

/// Why content admits no token estimate at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EstimateUnavailable {
    /// The bytes are not text: they do not decode as UTF-8.
    Undecodable,
    /// The bytes decode but carry binary structure a text heuristic cannot measure.
    BinaryLike,
}

/// The deterministic structural heuristic shipped in this phase.
///
/// The heuristic segments decoded text into word, whitespace, symbol, line-break, and non-ASCII
/// runs and charges each run a fixed structural cost. It is not a tokenizer and never claims to
/// be: every estimate it produces reports [`EstimateConfidence::Heuristic`] and leaves
/// [`TokenEstimate::encoding`] absent.
///
/// Two properties consumers may rely on: identical bytes always produce an identical estimate, and
/// appending bytes never lowers the estimate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuralHeuristicEstimator {
    identity: BoundedMetadataText,
}

impl StructuralHeuristicEstimator {
    /// Identity recorded with every estimate this estimator produces.
    pub const NAME: &'static str = "structural-heuristic";
    /// Version of this estimator's rules.
    pub const VERSION: u32 = 1;
    /// Word characters charged as one structural token.
    const WORD_CHARS_PER_TOKEN: u64 = 4;
    /// Consecutive spaces or tabs charged as one structural token.
    ///
    /// A single separating space is absorbed by the token that follows it, which is why the charge
    /// floors instead of rounding up; a run long enough to be indentation is charged.
    const SPACES_PER_TOKEN: u64 = 4;
    /// Encoded bytes of one non-ASCII character charged as one structural token.
    const NON_ASCII_BYTES_PER_TOKEN: u64 = 2;
    /// Reciprocal share of decoded characters that may be control characters before content is
    /// treated as binary rather than text.
    const CONTROL_SHARE_DIVISOR: u64 = 16;

    /// Creates the estimator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            identity: BoundedMetadataText::estimator_identity(Self::NAME),
        }
    }

    fn estimate_of(&self, tokens: u64) -> TokenEstimate {
        TokenEstimate {
            tokens,
            estimator: self.identity.clone(),
            estimator_version: Self::VERSION,
            encoding: None,
            confidence: EstimateConfidence::Heuristic,
        }
    }
}

impl Default for StructuralHeuristicEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenEstimator for StructuralHeuristicEstimator {
    fn identity(&self) -> &BoundedMetadataText {
        &self.identity
    }

    fn version(&self) -> u32 {
        Self::VERSION
    }

    /// The model name is deliberately unused: this heuristic has no model-to-encoding mapping, so
    /// pretending the model influenced the number would be the approximation the contract forbids.
    fn estimate(&self, request: &EstimationRequest<'_>) -> TokenEstimation {
        let Some(text) = admitted_text(request.content, &request.limits) else {
            return TokenEstimation::Unavailable(EstimateUnavailable::Undecodable);
        };
        let scan = scan_text(text.inspected);
        if scan.is_binary_like() {
            return TokenEstimation::Unavailable(EstimateUnavailable::BinaryLike);
        }
        let estimate = self.estimate_of(scan.tokens);
        if text.skipped_bytes == 0 {
            TokenEstimation::Complete(estimate)
        } else {
            TokenEstimation::Bounded {
                estimate,
                inspected_bytes: text.inspected_bytes,
                skipped_bytes: text.skipped_bytes,
            }
        }
    }
}

/// Unknown-preserving aggregation of per-block estimates.
///
/// Summing estimates is only honest when every contributing block had one. This aggregate keeps
/// the subtotal of the estimates it did see — work inside a bound is never discarded — while
/// refusing to present that subtotal as a total: [`Self::complete_total`] is absent as soon as one
/// observed block carried no estimate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokenEstimateAggregate {
    observed_blocks: u64,
    estimated_blocks: u64,
    bounded_blocks: u64,
    subtotal: u64,
    skipped_bytes: u64,
    saturated: bool,
    estimator: Option<BoundedMetadataText>,
    estimator_version: Option<u32>,
    mixed_estimators: bool,
}

impl TokenEstimateAggregate {
    /// Creates an empty aggregate.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            observed_blocks: 0,
            estimated_blocks: 0,
            bounded_blocks: 0,
            subtotal: 0,
            skipped_bytes: 0,
            saturated: false,
            estimator: None,
            estimator_version: None,
            mixed_estimators: false,
        }
    }

    /// Observes one block whose estimate may be absent.
    ///
    /// An absent estimate is recorded as an unestimated block rather than as zero tokens.
    pub fn observe_estimate(&mut self, estimate: Option<&TokenEstimate>) {
        self.observed_blocks = self.observed_blocks.saturating_add(1);
        let Some(estimate) = estimate else {
            return;
        };
        self.estimated_blocks = self.estimated_blocks.saturating_add(1);
        if let Some(subtotal) = self.subtotal.checked_add(estimate.tokens) {
            self.subtotal = subtotal;
        } else {
            self.subtotal = u64::MAX;
            self.saturated = true;
        }
        self.record_identity(estimate);
    }

    /// Observes one block's estimation outcome, including whether a bound truncated it.
    pub fn observe(&mut self, estimation: &TokenEstimation) {
        self.observe_estimate(estimation.estimate());
        if !estimation.is_complete() && estimation.estimate().is_some() {
            self.bounded_blocks = self.bounded_blocks.saturating_add(1);
        }
        self.skipped_bytes = self
            .skipped_bytes
            .saturating_add(estimation.skipped_bytes());
    }

    /// Returns the blocks observed.
    #[must_use]
    pub const fn observed_blocks(&self) -> u64 {
        self.observed_blocks
    }

    /// Returns the observed blocks that carried an estimate.
    #[must_use]
    pub const fn estimated_blocks(&self) -> u64 {
        self.estimated_blocks
    }

    /// Returns the observed blocks that carried no estimate.
    #[must_use]
    pub const fn unestimated_blocks(&self) -> u64 {
        self.observed_blocks.saturating_sub(self.estimated_blocks)
    }

    /// Returns the observed blocks whose estimate covered a bounded prefix only.
    #[must_use]
    pub const fn bounded_blocks(&self) -> u64 {
        self.bounded_blocks
    }

    /// Returns the content bytes bounds left uninspected across the observed blocks.
    #[must_use]
    pub const fn skipped_bytes(&self) -> u64 {
        self.skipped_bytes
    }

    /// Returns whether every observed block carried an estimate and the sum stayed representable.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        !self.saturated && self.observed_blocks == self.estimated_blocks
    }

    /// Returns the total estimated tokens, absent unless every observed block was estimated.
    ///
    /// Absent means unknown. A caller that wants the part that is known asks
    /// [`Self::estimated_subtotal`] and reads [`Self::unestimated_blocks`] alongside it.
    #[must_use]
    pub const fn complete_total(&self) -> Option<u64> {
        if self.is_complete() {
            Some(self.subtotal)
        } else {
            None
        }
    }

    /// Returns the sum of the estimates that were present.
    ///
    /// This is a subtotal over [`Self::estimated_blocks`], not a total over
    /// [`Self::observed_blocks`], and it saturates rather than wrapping.
    #[must_use]
    pub const fn estimated_subtotal(&self) -> u64 {
        self.subtotal
    }

    /// Returns whether the sum exceeded what a `u64` can represent.
    #[must_use]
    pub const fn saturated(&self) -> bool {
        self.saturated
    }

    /// Returns the estimator identity shared by every contributing estimate.
    ///
    /// Absent when nothing was estimated or when estimates from different estimators or versions
    /// were mixed, because one identity may not be attached to a mixed sum.
    #[must_use]
    pub const fn estimator(&self) -> Option<&BoundedMetadataText> {
        if self.mixed_estimators {
            None
        } else {
            self.estimator.as_ref()
        }
    }

    /// Returns the estimator version shared by every contributing estimate.
    #[must_use]
    pub const fn estimator_version(&self) -> Option<u32> {
        if self.mixed_estimators {
            None
        } else {
            self.estimator_version
        }
    }

    fn record_identity(&mut self, estimate: &TokenEstimate) {
        match &self.estimator {
            None => {
                self.estimator = Some(estimate.estimator.clone());
                self.estimator_version = Some(estimate.estimator_version);
            }
            Some(known) => {
                if *known != estimate.estimator
                    || self.estimator_version != Some(estimate.estimator_version)
                {
                    self.mixed_estimators = true;
                }
            }
        }
    }
}

/// The decoded prefix of content that the analysis limits admit.
struct AdmittedText<'content> {
    inspected: &'content str,
    inspected_bytes: u64,
    skipped_bytes: u64,
}

/// Decodes the admitted prefix of `content`, or reports that the bytes are not text.
///
/// The bound is applied before decoding, so cost is a function of the bound rather than of the
/// content length. A multi-byte character split by the bound is skipped rather than treated as
/// undecodable content; a genuinely malformed sequence inside the admitted prefix yields [`None`].
fn admitted_text<'content>(
    content: &'content [u8],
    limits: &ContextAnalysisLimits,
) -> Option<AdmittedText<'content>> {
    let bound = limits.max_analyzed_bytes.get();
    let (prefix, bounded) = content
        .get(..bound)
        .map_or((content, false), |prefix| (prefix, true));
    let inspected =
        match str::from_utf8(prefix) {
            Ok(text) => text,
            Err(error) if bounded && error.error_len().is_none() => prefix
                .get(..error.valid_up_to())
                .and_then(|valid| str::from_utf8(valid).ok())?,
            Err(_) => return None,
        };
    let inspected_bytes = byte_count(inspected.len());
    Some(AdmittedText {
        inspected,
        inspected_bytes,
        skipped_bytes: byte_count(content.len()).saturating_sub(inspected_bytes),
    })
}

/// Structural totals of one decoded prefix.
struct TextScan {
    tokens: u64,
    characters: u64,
    control_characters: u64,
    has_nul: bool,
}

impl TextScan {
    const fn is_binary_like(&self) -> bool {
        self.has_nul
            || self
                .control_characters
                .saturating_mul(StructuralHeuristicEstimator::CONTROL_SHARE_DIVISOR)
                > self.characters
    }
}

/// Which structural run a character belongs to.
enum CharClass {
    /// An ASCII letter, digit, or underscore.
    Word,
    /// A space or tab.
    Space,
    /// A line break or other control character.
    Break,
    /// Any other ASCII character.
    Symbol,
    /// A character outside ASCII, charged by its encoded length.
    NonAscii,
}

const fn classify(character: char) -> CharClass {
    if !character.is_ascii() {
        return CharClass::NonAscii;
    }
    if character.is_ascii_alphanumeric() || character == '_' {
        return CharClass::Word;
    }
    if character == ' ' || character == '\t' {
        return CharClass::Space;
    }
    if character.is_ascii_control() {
        return CharClass::Break;
    }
    CharClass::Symbol
}

/// Charges every structural run of `text` and totals the character statistics.
///
/// The scan is causal: a character's charge depends only on the characters before it and on the
/// length of the run it belongs to, and every charge is non-negative. That is what makes the
/// estimate deterministic for identical bytes and monotone under appending.
fn scan_text(text: &str) -> TextScan {
    let mut scan = TextScan {
        tokens: 0,
        characters: 0,
        control_characters: 0,
        has_nul: false,
    };
    let mut word_run: u64 = 0;
    let mut space_run: u64 = 0;

    for character in text.chars() {
        scan.characters = scan.characters.saturating_add(1);
        if character == '\0' {
            scan.has_nul = true;
        }
        if character.is_control() && !matches!(character, '\t' | '\n' | '\r') {
            scan.control_characters = scan.control_characters.saturating_add(1);
        }

        match classify(character) {
            CharClass::Word => {
                scan.tokens = flush_spaces(scan.tokens, &mut space_run);
                word_run = word_run.saturating_add(1);
            }
            CharClass::Space => {
                scan.tokens = flush_word(scan.tokens, &mut word_run);
                space_run = space_run.saturating_add(1);
            }
            CharClass::Break | CharClass::Symbol => {
                scan.tokens = flush_word(scan.tokens, &mut word_run);
                scan.tokens = flush_spaces(scan.tokens, &mut space_run);
                scan.tokens = scan.tokens.saturating_add(1);
            }
            CharClass::NonAscii => {
                scan.tokens = flush_word(scan.tokens, &mut word_run);
                scan.tokens = flush_spaces(scan.tokens, &mut space_run);
                let encoded = byte_count(character.len_utf8());
                scan.tokens = scan.tokens.saturating_add(
                    encoded.div_ceil(StructuralHeuristicEstimator::NON_ASCII_BYTES_PER_TOKEN),
                );
            }
        }
    }

    scan.tokens = flush_word(scan.tokens, &mut word_run);
    scan.tokens = flush_spaces(scan.tokens, &mut space_run);
    scan
}

const fn flush_word(tokens: u64, word_run: &mut u64) -> u64 {
    let run = core::mem::replace(word_run, 0);
    if run == 0 {
        return tokens;
    }
    tokens.saturating_add(run.div_ceil(StructuralHeuristicEstimator::WORD_CHARS_PER_TOKEN))
}

fn flush_spaces(tokens: u64, space_run: &mut u64) -> u64 {
    let run = core::mem::replace(space_run, 0);
    tokens.saturating_add(
        run.checked_div(StructuralHeuristicEstimator::SPACES_PER_TOKEN)
            .unwrap_or(0),
    )
}

fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}
