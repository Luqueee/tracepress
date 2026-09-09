//! Observable contract of local token estimation and its unknown-preserving aggregation.

use tracepress_context::{
    BoundedMetadataText, ContextAnalysisLimitValues, ContextAnalysisLimits,
    ContextAnalysisLimitsError, EstimateConfidence, EstimateUnavailable, EstimationRequest,
    StructuralHeuristicEstimator, TokenEstimate, TokenEstimateAggregate, TokenEstimation,
    TokenEstimator,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PROSE: &str = "The quick brown fox jumps over the lazy dog, repeatedly and deliberately.";

/// Builds one estimation request over borrowed content.
const fn ask<'content>(
    model: Option<&'content str>,
    content: &'content [u8],
    limits: &ContextAnalysisLimits,
) -> EstimationRequest<'content> {
    EstimationRequest {
        model,
        content,
        limits: *limits,
    }
}

#[test]
fn identical_bytes_produce_identical_estimates() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let other = StructuralHeuristicEstimator::default();

    let first = estimator.estimate(&ask(Some("gpt-4.1"), PROSE.as_bytes(), &limits));
    let second = estimator.estimate(&ask(Some("gpt-4.1"), PROSE.as_bytes(), &limits));
    let third = other.estimate(&ask(None, PROSE.as_bytes(), &limits));

    assert_eq!(first, second);
    assert_eq!(first, third);
    Ok(())
}

#[test]
fn an_estimate_never_claims_to_be_exact_or_model_mapped() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let contents: [&[u8]; 4] = [
        PROSE.as_bytes(),
        br#"{"tool":"grep","args":{"pattern":"foo"}}"#,
        b"  indented\n\tlines\n",
        "日本語のテキストと emoji 🎉".as_bytes(),
    ];

    for content in contents {
        let estimation = estimator.estimate(&ask(Some("gpt-4.1"), content, &limits));
        let estimate = estimation
            .estimate()
            .ok_or("expected an estimate for text content")?;
        assert_eq!(estimate.confidence, EstimateConfidence::Heuristic);
        assert_eq!(
            estimate.estimator.as_str(),
            StructuralHeuristicEstimator::NAME
        );
        assert_eq!(estimate.estimator_version, estimator.version());
        assert_eq!(estimate.estimator.as_str(), estimator.identity().as_str());
        assert!(
            estimate.encoding.is_none(),
            "no model-to-encoding mapping exists in this phase"
        );
    }
    Ok(())
}

#[test]
fn undecodable_content_yields_no_estimate_rather_than_zero() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();

    let estimation = estimator.estimate(&ask(None, &[0xff, 0xfe, 0x41, 0x80, 0x42], &limits));

    assert_eq!(estimation.tokens(), None);
    assert_eq!(estimation.estimate(), None);
    assert_eq!(
        estimation.unavailable_reason(),
        Some(EstimateUnavailable::Undecodable)
    );
    Ok(())
}

#[test]
fn binary_like_content_yields_no_estimate_rather_than_zero() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let mut content = b"header".to_vec();
    content.extend_from_slice(&[0x00, 0x01, 0x02, 0x00, 0x03]);

    let estimation = estimator.estimate(&ask(None, &content, &limits));

    assert_eq!(estimation.tokens(), None);
    assert_eq!(
        estimation.unavailable_reason(),
        Some(EstimateUnavailable::BinaryLike)
    );
    Ok(())
}

#[test]
fn dense_control_bytes_are_not_estimated_as_text() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let content: Vec<u8> = (0..64_u8).map(|index| 0x01 + (index % 7)).collect();

    let estimation = estimator.estimate(&ask(None, &content, &limits));

    assert_eq!(estimation.tokens(), None);
    assert_eq!(
        estimation.unavailable_reason(),
        Some(EstimateUnavailable::BinaryLike)
    );
    Ok(())
}

#[test]
fn empty_content_is_a_known_zero_and_not_an_unknown() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();

    let estimation = estimator.estimate(&ask(None, b"", &limits));

    assert!(estimation.is_complete());
    assert_eq!(estimation.tokens(), Some(0));
    Ok(())
}

#[test]
fn appending_content_never_lowers_the_estimate() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let text = format!("{PROSE}\n  {{\"nested\": [1, 2, 3]}}\n日本語 tail");

    let mut previous = 0_u64;
    for boundary in text
        .char_indices()
        .map(|(index, _)| index)
        .chain([text.len()])
    {
        let prefix = text
            .get(..boundary)
            .ok_or("expected a character boundary")?;
        let tokens = estimator
            .estimate(&ask(None, prefix.as_bytes(), &limits))
            .tokens()
            .ok_or("expected an estimate for text content")?;
        assert!(
            tokens >= previous,
            "estimate fell from {previous} to {tokens} after appending bytes"
        );
        previous = tokens;
    }
    assert!(previous > 0, "non-empty text must estimate above zero");
    Ok(())
}

#[test]
fn an_ascii_estimate_stays_within_the_hard_token_bounds_of_its_length() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let characters = u64::try_from(PROSE.chars().count())?;

    let tokens = estimator
        .estimate(&ask(None, PROSE.as_bytes(), &limits))
        .tokens()
        .ok_or("expected an estimate for text content")?;

    assert!(
        tokens > 0 && tokens <= characters,
        "{tokens} tokens is outside the 1..={characters} band an ASCII text admits"
    );
    Ok(())
}

#[test]
fn a_very_large_input_is_inspected_only_within_the_byte_bound() -> TestResult {
    let bound = 1_024_u64;
    let limits = limits_with_analyzed_bytes(bound)?;
    let estimator = StructuralHeuristicEstimator::new();
    let content = PROSE.repeat(40_000);
    let content_bytes = u64::try_from(content.len())?;
    assert!(content_bytes > bound * 1_000, "input must dwarf the bound");

    let estimation = estimator.estimate(&ask(None, content.as_bytes(), &limits));

    let TokenEstimation::Bounded {
        estimate,
        inspected_bytes,
        skipped_bytes,
    } = &estimation
    else {
        return Err("an input above the byte bound must report a bounded estimate".into());
    };
    assert_eq!(*inspected_bytes, bound);
    assert_eq!(
        inspected_bytes.saturating_add(*skipped_bytes),
        content_bytes,
        "every content byte is either inspected or reported as skipped"
    );
    assert!(
        estimate.tokens > 0 && estimate.tokens <= bound,
        "the work completed inside the bound is kept and never exceeds it"
    );
    assert!(!estimation.is_complete());
    Ok(())
}

#[test]
fn a_bound_splitting_a_multibyte_character_still_estimates() -> TestResult {
    let limits = limits_with_analyzed_bytes(10)?;
    let estimator = StructuralHeuristicEstimator::new();
    // Four three-byte characters: the bound falls one byte inside the fourth.
    let content = "日本語だ";

    let estimation = estimator.estimate(&ask(None, content.as_bytes(), &limits));

    assert!(
        estimation.estimate().is_some(),
        "a character split by the bound is skipped, not treated as undecodable"
    );
    assert_eq!(estimation.skipped_bytes(), 3);
    Ok(())
}

#[test]
fn an_aggregate_with_one_missing_estimate_is_not_a_complete_total() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let present = estimator.estimate(&ask(None, PROSE.as_bytes(), &limits));
    let missing = estimator.estimate(&ask(None, &[0xff, 0xff], &limits));
    let known = present
        .tokens()
        .ok_or("expected an estimate for text content")?;

    let mut aggregate = TokenEstimateAggregate::new();
    aggregate.observe(&present);
    aggregate.observe(&missing);
    aggregate.observe(&present);

    assert_eq!(aggregate.complete_total(), None);
    assert!(!aggregate.is_complete());
    assert_eq!(aggregate.observed_blocks(), 3);
    assert_eq!(aggregate.estimated_blocks(), 2);
    assert_eq!(aggregate.unestimated_blocks(), 1);
    assert_eq!(
        aggregate.estimated_subtotal(),
        known.saturating_mul(2),
        "the work already completed is kept as a subtotal"
    );
    Ok(())
}

#[test]
fn an_aggregate_over_estimated_blocks_totals_them() -> TestResult {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    let estimator = StructuralHeuristicEstimator::new();
    let estimation = estimator.estimate(&ask(None, PROSE.as_bytes(), &limits));
    let tokens = estimation
        .tokens()
        .ok_or("expected an estimate for text content")?;

    let mut aggregate = TokenEstimateAggregate::new();
    aggregate.observe(&estimation);
    aggregate.observe(&estimation);

    assert!(aggregate.is_complete());
    assert_eq!(aggregate.complete_total(), Some(tokens.saturating_mul(2)));
    assert_eq!(aggregate.unestimated_blocks(), 0);
    assert_eq!(
        aggregate.estimator().map(BoundedMetadataText::as_str),
        Some(StructuralHeuristicEstimator::NAME)
    );
    assert_eq!(
        aggregate.estimator_version(),
        Some(StructuralHeuristicEstimator::VERSION)
    );
    Ok(())
}

#[test]
fn an_empty_aggregate_reports_a_total_over_nothing() {
    let aggregate = TokenEstimateAggregate::new();

    assert_eq!(aggregate.complete_total(), Some(0));
    assert_eq!(aggregate.observed_blocks(), 0);
    assert_eq!(aggregate.estimator(), None);
    assert_eq!(aggregate.estimator_version(), None);
}

#[test]
fn an_aggregate_records_bounded_estimates_and_their_skipped_bytes() -> TestResult {
    let limits = limits_with_analyzed_bytes(64)?;
    let estimator = StructuralHeuristicEstimator::new();
    let content = PROSE.repeat(4);
    let bounded = estimator.estimate(&ask(None, content.as_bytes(), &limits));
    let complete = estimator.estimate(&ask(None, b"short", &limits));

    let mut aggregate = TokenEstimateAggregate::new();
    aggregate.observe(&bounded);
    aggregate.observe(&complete);

    assert!(
        aggregate.is_complete(),
        "every observed block carried an estimate"
    );
    assert_eq!(aggregate.bounded_blocks(), 1);
    assert_eq!(aggregate.skipped_bytes(), bounded.skipped_bytes());
    assert!(aggregate.skipped_bytes() > 0);
    Ok(())
}

#[test]
fn an_aggregate_of_mixed_estimators_attaches_no_single_identity() -> TestResult {
    let base = seed_estimate()?;

    let mut aggregate = TokenEstimateAggregate::new();
    aggregate.observe_estimate(Some(&relabelled(
        &base,
        EstimateFixture::new("structural-heuristic", 1, 10),
    )));
    aggregate.observe_estimate(Some(&relabelled(
        &base,
        EstimateFixture::new("other-estimator", 1, 5),
    )));

    assert_eq!(aggregate.complete_total(), Some(15));
    assert_eq!(aggregate.estimator(), None);
    assert_eq!(aggregate.estimator_version(), None);
    Ok(())
}

#[test]
fn an_aggregate_of_mixed_estimator_versions_attaches_no_single_identity() -> TestResult {
    let base = seed_estimate()?;

    let mut aggregate = TokenEstimateAggregate::new();
    aggregate.observe_estimate(Some(&relabelled(
        &base,
        EstimateFixture::new("structural-heuristic", 1, 10),
    )));
    aggregate.observe_estimate(Some(&relabelled(
        &base,
        EstimateFixture::new("structural-heuristic", 2, 5),
    )));

    assert_eq!(aggregate.estimator_version(), None);
    assert_eq!(aggregate.estimator(), None);
    Ok(())
}

#[test]
fn an_unrepresentable_sum_is_unknown_rather_than_wrong() -> TestResult {
    let base = seed_estimate()?;

    let mut aggregate = TokenEstimateAggregate::new();
    aggregate.observe_estimate(Some(&relabelled(
        &base,
        EstimateFixture::new(
            StructuralHeuristicEstimator::NAME,
            StructuralHeuristicEstimator::VERSION,
            u64::MAX - 1,
        ),
    )));
    aggregate.observe_estimate(Some(&relabelled(
        &base,
        EstimateFixture::new(
            StructuralHeuristicEstimator::NAME,
            StructuralHeuristicEstimator::VERSION,
            8,
        ),
    )));

    assert!(aggregate.saturated());
    assert_eq!(aggregate.estimated_subtotal(), u64::MAX);
    assert_eq!(aggregate.complete_total(), None);
    assert!(!aggregate.is_complete());
    Ok(())
}

#[test]
fn an_absent_block_estimate_is_never_read_as_zero_tokens() {
    let mut aggregate = TokenEstimateAggregate::new();
    aggregate.observe_estimate(None);

    assert_eq!(aggregate.complete_total(), None);
    assert_eq!(aggregate.estimated_subtotal(), 0);
    assert_eq!(aggregate.unestimated_blocks(), 1);
}

/// An estimate the shipped estimator really produced, used as the base of relabelled fixtures.
fn seed_estimate() -> Result<TokenEstimate, Box<dyn std::error::Error>> {
    let limits = limits_with_analyzed_bytes(ContextAnalysisLimits::ANALYZED_BYTES_CEILING)?;
    StructuralHeuristicEstimator::new()
        .estimate(&ask(None, b"seed", &limits))
        .estimate()
        .cloned()
        .ok_or_else(|| "expected an estimate for text content".into())
}

#[derive(Clone, Copy)]
struct EstimateFixture<'name> {
    name: &'name str,
    version: u32,
    tokens: u64,
}

impl<'name> EstimateFixture<'name> {
    const fn new(name: &'name str, version: u32, tokens: u64) -> Self {
        Self {
            name,
            version,
            tokens,
        }
    }
}

fn relabelled(base: &TokenEstimate, fixture: EstimateFixture<'_>) -> TokenEstimate {
    let mut estimate = base.clone();
    estimate.tokens = fixture.tokens;
    estimate.estimator = BoundedMetadataText::estimator_identity(fixture.name);
    estimate.estimator_version = fixture.version;
    estimate
}

fn limits_with_analyzed_bytes(
    max_analyzed_bytes: u64,
) -> Result<ContextAnalysisLimits, ContextAnalysisLimitsError> {
    ContextAnalysisLimits::new(ContextAnalysisLimitValues {
        max_analyzed_bytes,
        max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
        max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
        max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        max_analysis_work_units: 2_000,
        max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
        max_batches: ContextAnalysisLimits::MAX_BATCHES,
    })
}
