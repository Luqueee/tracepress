//! Observable contract of the structural shadow content detectors.

use tracepress_context::{
    BlockContentMetadata, ContextAnalysisLimitValues, ContextAnalysisLimits, ContextBlockKind,
    DetectedContentKind, DetectionConfidence, DetectionResult, STRUCTURAL_DETECTOR_VERSION,
    ShadowContentDetector, StructuralContentDetector,
};

const JSON_DOCUMENT: &str = r#"{
  "model": "gpt-5",
  "input": [{"role": "user", "content": "summarise the design review"}],
  "tools": [],
  "max_output_tokens": 2048
}
"#;

const NDJSON_STREAM: &str = r#"{"ts":"2026-03-04T09:12:33Z","event":"start","attempt":1}
{"ts":"2026-03-04T09:12:34Z","event":"retry","attempt":2}
{"ts":"2026-03-04T09:12:35Z","event":"done","attempt":3}
"#;

const LOG_EXCERPT: &str = "2026-03-04T09:12:33.881Z INFO  worker=3 pool ready
2026-03-04T09:12:34.002Z DEBUG worker=3 fetch begin bytes=4096
2026-03-04T09:12:35.117Z WARN  worker=3 retry scheduled attempt=2
2026-03-04T09:12:36.900Z ERROR worker=3 upstream timeout after 1500ms
";

const SEARCH_RESULTS: &str = "crates/tracepress-context/src/detector.rs:118:    pub const fn new(limits: &ContextAnalysisLimits) -> Self {
crates/tracepress-context/src/limits.rs:96:    pub const MAX_STRING_BYTES_INSPECTED: u64 = 65_536;
crates/tracepress-proxy/src/forward.rs:204:    let accepted = body.freeze();
";

const TEST_RUNNER_OUTPUT: &str = "running 4 tests
test detector::a_json_document_is_recognised ... ok
test detector::an_ndjson_stream_is_recognised ... ok
test detector::binary_bytes_are_their_own_kind ... FAILED

test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured
";

const UNIFIED_DIFF: &str = "diff --git a/crates/tracepress-context/src/detector.rs b/crates/tracepress-context/src/detector.rs
--- a/crates/tracepress-context/src/detector.rs
+++ b/crates/tracepress-context/src/detector.rs
@@ -118,7 +118,9 @@ impl StructuralContentDetector {
     pub const fn new(limits: &ContextAnalysisLimits) -> Self {
-        let configured = limits.max_json_depth.get();
+        let configured_depth = limits.max_json_depth.get();
+        let budget = limits.max_string_bytes_inspected;
";

const RUST_SOURCE: &str = "use std::num::NonZeroUsize;

pub fn inspected_prefix(content: &[u8], budget: NonZeroUsize) -> &[u8] {
    let limit = budget.get();
    match content.get(..limit) {
        Some(prefix) => prefix,
        None => content,
    }
}
";

const PYTHON_SOURCE: &str = "import json

def load_manifest(path):
    with open(path, encoding=\"utf-8\") as handle:
        return json.load(handle)
";

const PROSE: &str =
    "The proxy forwards every accepted request byte for byte, and the analyzer never
touches the buffer it reads. When a bound is crossed, the work already finished
is kept rather than discarded, because a partial answer that is honest about its
own limits is more useful than a confident answer that is quietly wrong.
";

const PNG_PREFIX: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x01\x00\x00\x00\x01\x00\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

const INVALID_UTF8: &[u8] = b"\xff\xfe\x41\x42\xc3\x28\x9f\x80\xed\xa0\x80";

const OPAQUE_TOKEN: &str = "aGVsbG8sIHNoYWRvdyBjb250ZXh0IGFuYWx5c2lz";

const NUMERIC_COLUMN: &str = "17\n42\n8\n129\n";

const JSON_FRAGMENT: &str = r#"{"model": "gpt-5", "input": [{"role": "user", "content": "sum"#;

/// One realistic sample and the verdict its shape licenses.
struct DetectionCase {
    name: &'static str,
    content: &'static [u8],
    kind: DetectedContentKind,
    minimum_confidence: DetectionConfidence,
}

const CASES: [DetectionCase; 15] = [
    DetectionCase {
        name: "a responses request document",
        content: JSON_DOCUMENT.as_bytes(),
        kind: DetectedContentKind::Json,
        minimum_confidence: DetectionConfidence::High,
    },
    DetectionCase {
        name: "a newline-delimited event stream",
        content: NDJSON_STREAM.as_bytes(),
        kind: DetectedContentKind::Ndjson,
        minimum_confidence: DetectionConfidence::High,
    },
    DetectionCase {
        name: "a timestamped and levelled log excerpt",
        content: LOG_EXCERPT.as_bytes(),
        kind: DetectedContentKind::Log,
        minimum_confidence: DetectionConfidence::High,
    },
    DetectionCase {
        name: "grep-style search results",
        content: SEARCH_RESULTS.as_bytes(),
        kind: DetectedContentKind::SearchResults,
        minimum_confidence: DetectionConfidence::High,
    },
    DetectionCase {
        name: "test-runner output with a summary",
        content: TEST_RUNNER_OUTPUT.as_bytes(),
        kind: DetectedContentKind::TestResults,
        minimum_confidence: DetectionConfidence::Medium,
    },
    DetectionCase {
        name: "a unified diff",
        content: UNIFIED_DIFF.as_bytes(),
        kind: DetectedContentKind::Diff,
        minimum_confidence: DetectionConfidence::High,
    },
    DetectionCase {
        name: "rust source",
        content: RUST_SOURCE.as_bytes(),
        kind: DetectedContentKind::SourceCode,
        minimum_confidence: DetectionConfidence::Medium,
    },
    DetectionCase {
        name: "python source",
        content: PYTHON_SOURCE.as_bytes(),
        kind: DetectedContentKind::SourceCode,
        minimum_confidence: DetectionConfidence::Medium,
    },
    DetectionCase {
        name: "prose",
        content: PROSE.as_bytes(),
        kind: DetectedContentKind::PlainText,
        minimum_confidence: DetectionConfidence::Medium,
    },
    DetectionCase {
        name: "a png header carrying nul bytes",
        content: PNG_PREFIX,
        kind: DetectedContentKind::BinaryLike,
        minimum_confidence: DetectionConfidence::High,
    },
    DetectionCase {
        name: "bytes that are not utf-8",
        content: INVALID_UTF8,
        kind: DetectedContentKind::BinaryLike,
        minimum_confidence: DetectionConfidence::High,
    },
    DetectionCase {
        name: "one opaque base64 token",
        content: OPAQUE_TOKEN.as_bytes(),
        kind: DetectedContentKind::Unknown,
        minimum_confidence: DetectionConfidence::Low,
    },
    DetectionCase {
        name: "a column of bare numbers",
        content: NUMERIC_COLUMN.as_bytes(),
        kind: DetectedContentKind::Unknown,
        minimum_confidence: DetectionConfidence::Low,
    },
    DetectionCase {
        name: "a json document cut mid-string",
        content: JSON_FRAGMENT.as_bytes(),
        kind: DetectedContentKind::Unknown,
        minimum_confidence: DetectionConfidence::Low,
    },
    DetectionCase {
        name: "no content at all",
        content: &[],
        kind: DetectedContentKind::Unknown,
        minimum_confidence: DetectionConfidence::Low,
    },
];

#[test]
fn realistic_content_is_classified_as_its_shape_or_abstained_on()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;

    for case in &CASES {
        // When
        let result = detector.detect(case.content, text_block());

        // Then
        assert_eq!(
            result.kind, case.kind,
            "{} must be {:?}",
            case.name, case.kind
        );
        assert!(
            result.confidence >= case.minimum_confidence,
            "{} must be detected with at least {:?}, got {:?}",
            case.name,
            case.minimum_confidence,
            result.confidence
        );
        assert_eq!(
            result.detector_version, STRUCTURAL_DETECTOR_VERSION,
            "{} must record the detector version",
            case.name
        );
    }
    Ok(())
}

#[test]
fn a_low_confidence_verdict_is_always_an_abstention() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;

    for case in &CASES {
        // When
        let result = detector.detect(case.content, text_block());

        // Then
        assert_eq!(
            result.confidence == DetectionConfidence::Low,
            result.kind == DetectedContentKind::Unknown,
            "{} must pair low confidence with an unknown kind and nothing else",
            case.name
        );
    }
    Ok(())
}

#[test]
fn detection_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let one = detector()?;
    let other = detector()?;

    // When
    let first: Vec<DetectionResult> = CASES
        .iter()
        .map(|case| one.detect(case.content, text_block()))
        .collect();
    let reversed: Vec<DetectionResult> = CASES
        .iter()
        .rev()
        .map(|case| other.detect(case.content, text_block()))
        .collect();

    // Then
    let mut reversed_back = reversed;
    reversed_back.reverse();
    assert_eq!(
        first, reversed_back,
        "the same bytes must detect identically regardless of order or detector instance"
    );
    Ok(())
}

#[test]
fn a_reference_block_is_never_classified_by_its_own_envelope()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;
    let unreadable = [
        ContextBlockKind::FileReference,
        ContextBlockKind::ImageReference,
        ContextBlockKind::ItemReference,
        ContextBlockKind::PromptReference,
        ContextBlockKind::ProviderStateReference,
        ContextBlockKind::Opaque,
        ContextBlockKind::OpaqueReasoning,
    ];

    for kind in unreadable {
        // When
        let result = detector.detect(JSON_DOCUMENT.as_bytes(), BlockContentMetadata::new(kind));

        // Then
        assert_eq!(
            result.kind,
            DetectedContentKind::Unknown,
            "a {kind:?} block must abstain rather than describe its reference envelope"
        );
        assert_eq!(result.confidence, DetectionConfidence::Low);
    }
    Ok(())
}

#[test]
fn a_verdict_reached_from_a_prefix_is_never_high() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;
    let content = JSON_DOCUMENT.as_bytes();
    let declared = u64::try_from(content.len())?.saturating_add(4096);

    // When
    let whole = detector.detect(content, text_block());
    let prefix = detector.detect(
        content,
        BlockContentMetadata::new(ContextBlockKind::ToolResult).with_content_bytes(declared),
    );

    // Then
    assert_eq!(whole.confidence, DetectionConfidence::High);
    assert_eq!(prefix.kind, DetectedContentKind::Json);
    assert_eq!(
        prefix.confidence,
        DetectionConfidence::Medium,
        "content the caller already truncated may not be described with full confidence"
    );
    Ok(())
}

#[test]
fn detection_reads_no_more_than_the_inspection_budget() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;
    let budget = detector.inspected_bytes_budget();
    let mut content = Vec::new();
    while content.len() <= budget {
        content.extend_from_slice(b"{\"event\":\"tick\",\"attempt\":1}\n");
    }
    let inspected_prefix_len = content.len();
    content.extend(std::iter::repeat_n(0_u8, 4 * 1024 * 1024));

    // When
    let result = detector.detect(&content, text_block());

    // Then
    assert!(
        inspected_prefix_len > budget,
        "the sample must exceed the budget"
    );
    assert_eq!(
        result.kind,
        DetectedContentKind::Ndjson,
        "content beyond the budget must not participate in the verdict"
    );
    assert_eq!(
        result.confidence,
        DetectionConfidence::Medium,
        "a verdict about a prefix of a four megabyte payload may not be high"
    );
    Ok(())
}

#[test]
fn a_nul_byte_inside_the_budget_is_its_own_kind() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;
    let mut content = Vec::from(NDJSON_STREAM);
    content.push(0);

    // When
    let result = detector.detect(&content, text_block());

    // Then
    assert_eq!(result.kind, DetectedContentKind::BinaryLike);
    assert_eq!(result.confidence, DetectionConfidence::High);
    Ok(())
}

#[test]
fn a_json_document_larger_than_the_budget_is_reported_as_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;
    let budget = detector.inspected_bytes_budget();
    let mut content = Vec::from(b"[".as_slice());
    while content.len() <= budget {
        content.extend_from_slice(b"{\"role\":\"user\",\"content\":\"review the plan\"},");
    }
    content.extend_from_slice(b"{\"role\":\"user\",\"content\":\"done\"}]");

    // When
    let result = detector.detect(&content, text_block());

    // Then
    assert_eq!(result.kind, DetectedContentKind::Json);
    assert_eq!(
        result.confidence,
        DetectionConfidence::Medium,
        "a document whose end was never inspected may not be reported with full confidence"
    );
    Ok(())
}

#[test]
fn nesting_deeper_than_the_inspected_depth_abstains() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;
    let depth = 128;
    let mut content = Vec::new();
    content.extend(std::iter::repeat_n(b'[', depth));
    content.extend(std::iter::repeat_n(b']', depth));

    // When
    let result = detector.detect(&content, text_block());

    // Then
    assert_eq!(
        result.kind,
        DetectedContentKind::Unknown,
        "a shape the depth bound stopped short of must be unknown, not guessed"
    );
    assert_eq!(result.confidence, DetectionConfidence::Low);
    Ok(())
}

#[test]
fn every_mutation_of_the_corpus_is_answered() -> Result<(), Box<dyn std::error::Error>> {
    // Given
    let detector = detector()?;
    let mut noise = 0x2b99_2ddf_a232_4dc9_u64;
    let interesting = [0_u8, 0x1b, 0x7f, 0xff, b'"', b'\\', b'{', b'[', b':', b'\n'];

    for case in &CASES {
        for round in 0..64_u32 {
            let mut mutated = Vec::from(case.content);
            for _ in 0..=round {
                noise = noise
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let choice = usize::try_from(noise >> 33)?;
                let byte = interesting
                    .get(choice.checked_rem(interesting.len()).unwrap_or(0))
                    .copied()
                    .unwrap_or(b'?');
                let position = usize::try_from(noise >> 13)?
                    .checked_rem(mutated.len())
                    .unwrap_or(0);
                match mutated.get_mut(position) {
                    Some(slot) => *slot = byte,
                    None => mutated.push(byte),
                }
            }

            // When
            let result = detector.detect(&mutated, text_block());

            // Then
            assert_eq!(
                result.confidence == DetectionConfidence::Low,
                result.kind == DetectedContentKind::Unknown,
                "mutated {} must still pair low confidence with an unknown kind only",
                case.name
            );
            assert_eq!(result.detector_version, STRUCTURAL_DETECTOR_VERSION);
            assert_eq!(
                result,
                detector.detect(&mutated, text_block()),
                "mutated {} must detect identically twice",
                case.name
            );
        }
    }
    Ok(())
}

fn detector() -> Result<StructuralContentDetector, Box<dyn std::error::Error>> {
    Ok(StructuralContentDetector::new(&ContextAnalysisLimits::new(
        ContextAnalysisLimitValues {
            max_analyzed_bytes: ContextAnalysisLimits::ANALYZED_BYTES_CEILING,
            max_blocks: ContextAnalysisLimits::MAX_BLOCKS,
            max_json_depth: ContextAnalysisLimits::MAX_JSON_DEPTH,
            max_string_bytes_inspected: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
            max_analysis_work_units: 2_000,
            max_analysis_wall_time_ms: ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS,
            max_batches: ContextAnalysisLimits::MAX_BATCHES,
        },
    )?))
}

const fn text_block() -> BlockContentMetadata {
    BlockContentMetadata::new(ContextBlockKind::ToolResult)
}
