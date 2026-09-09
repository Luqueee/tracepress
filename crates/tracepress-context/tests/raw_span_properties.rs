//! Property invariants of the raw span index over generated and hostile documents.

use proptest::{
    prelude::*,
    test_runner::{Config, RngAlgorithm, RngSeed, TestCaseError},
};
use serde_json::Value;
use tracepress_context::{
    AnalysisClock, ContextAnalysisLimitValues, ContextAnalysisLimits, ContextAnalysisLimitsError,
    ContextAnalysisStatus, JsonValueKind, RawSpanIndex, SpanNode, decode_json_string,
};

include!("support/span_fixtures.rs");

const PROPTEST_CASES: u32 = 96;

fn deterministic_proptest_config() -> Config {
    Config {
        cases: PROPTEST_CASES,
        rng_algorithm: RngAlgorithm::ChaCha,
        rng_seed: RngSeed::Fixed(0x5350_414e_5f49_4458),
        ..Config::default()
    }
}

proptest! {
    #![proptest_config(deterministic_proptest_config())]

    #[test]
    fn generated_documents_keep_every_span_invariant(document in json_document()) {
        // Given
        let bytes = document.as_bytes();

        // When
        let index = index_document(&document)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // Then
        prop_assert!(
            matches!(
                index.status(),
                ContextAnalysisStatus::Complete | ContextAnalysisStatus::Partial
            ),
            "valid JSON must index, found {:?}",
            index.status()
        );
        prop_assert_eq!(index.duplicate_key_detected(), index.status() == ContextAnalysisStatus::Partial);
        prop_assert_eq!(index.analyzed_bytes(), u64::try_from(document.len()).unwrap_or(u64::MAX));
        prop_assert_eq!(index.skipped_bytes(), 0);
        let root = index.root().ok_or_else(|| TestCaseError::fail("no root value"))?;
        prop_assert!(root.is_complete());
        assert_spans_are_exact(&index, bytes, true)?;
        assert_structure_is_nested(&index)?;
        assert_root_is_the_whole_document(&index, bytes)?;
    }

    #[test]
    fn indexing_the_same_bytes_twice_yields_the_same_index(document in json_document()) {
        // When
        let first = index_document(&document)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let second = index_document(&document)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // Then
        prop_assert_eq!(first.nodes(), second.nodes());
        prop_assert_eq!(first.status(), second.status());
    }

    #[test]
    fn generated_documents_stay_bounded_under_tight_bounds(
        document in json_document(),
        max_blocks in 1_u64..24,
        max_json_depth in 1_u64..6,
        max_analyzed_bytes in 1_u64..96,
    ) {
        // Given
        let limits = tight_limits(TightBounds {
            blocks: max_blocks,
            json_depth: max_json_depth,
            analyzed_bytes: max_analyzed_bytes,
            string_bytes_inspected: 8,
        }).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let bytes = document.as_bytes();

        // When
        let index = RawSpanIndex::build_with_clock(bytes, limits, &FrozenClock);

        // Then
        prop_assert!(index.len() <= usize::try_from(max_blocks).unwrap_or(usize::MAX));
        for node in index.nodes() {
            prop_assert!(u64::from(node.depth()) <= max_json_depth);
        }
        prop_assert_eq!(
            index.analyzed_bytes().saturating_add(index.skipped_bytes()),
            u64::try_from(document.len()).unwrap_or(u64::MAX)
        );
        prop_assert!(index.analyzed_bytes() <= max_analyzed_bytes);
        assert_spans_are_exact(&index, bytes, false)?;
        assert_structure_is_nested(&index)?;
    }

    #[test]
    fn arbitrary_bytes_are_indexed_without_panic_or_mutation(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
        max_blocks in 1_u64..64,
        max_json_depth in 1_u64..8,
    ) {
        // Given
        let original = bytes.clone();
        let limits = tight_limits(TightBounds {
            blocks: max_blocks,
            json_depth: max_json_depth,
            analyzed_bytes: 512,
            string_bytes_inspected: 64,
        }).map_err(|error| TestCaseError::fail(error.to_string()))?;

        // When
        let index = RawSpanIndex::build_with_clock(&bytes, limits, &FrozenClock);

        // Then
        prop_assert_eq!(&bytes, &original);
        prop_assert!(matches!(
            index.status(),
            ContextAnalysisStatus::Complete
                | ContextAnalysisStatus::Partial
                | ContextAnalysisStatus::ResourceLimit
                | ContextAnalysisStatus::Malformed
        ));
        prop_assert_eq!(
            index.status() == ContextAnalysisStatus::ResourceLimit,
            index.limit_reached().is_some()
        );
        prop_assert_eq!(
            index.analyzed_bytes().saturating_add(index.skipped_bytes()),
            u64::try_from(bytes.len()).unwrap_or(u64::MAX)
        );
        assert_spans_are_exact(&index, &bytes, false)?;
        assert_structure_is_nested(&index)?;
    }
}

/// Bounds a property case tightens one at a time.
#[derive(Clone, Copy, Debug)]
struct TightBounds {
    blocks: u64,
    json_depth: u64,
    analyzed_bytes: u64,
    string_bytes_inspected: u64,
}

fn tight_limits(bounds: TightBounds) -> Result<ContextAnalysisLimits, ContextAnalysisLimitsError> {
    let mut values = analysis_values();
    values.max_blocks = bounds.blocks;
    values.max_json_depth = bounds.json_depth;
    values.max_analyzed_bytes = bounds.analyzed_bytes;
    values.max_string_bytes_inspected = bounds.string_bytes_inspected;
    ContextAnalysisLimits::new(values)
}

/// Asserts that every complete span is exactly one JSON value of the reported kind.
fn assert_spans_are_exact(
    index: &RawSpanIndex,
    request: &[u8],
    decodes_within_bound: bool,
) -> Result<(), TestCaseError> {
    let length = u64::try_from(request.len()).unwrap_or(u64::MAX);
    for node in index.nodes() {
        let span = node.span();
        prop_assert!(span.start() <= span.end(), "an inverted span was reported");
        prop_assert!(span.end() <= length, "a span reached past the request");
        let Some(slice) = span.slice(request) else {
            return Err(TestCaseError::fail("a reported span could not be sliced"));
        };
        if !node.is_complete() {
            continue;
        }
        let (Some(first), Some(last)) = (slice.first(), slice.last()) else {
            return Err(TestCaseError::fail("a complete value spanned no bytes"));
        };
        prop_assert!(
            !is_json_whitespace(*first) && !is_json_whitespace(*last),
            "a span included surrounding whitespace"
        );
        if let Ok(value) = serde_json::from_slice::<Value>(slice) {
            prop_assert!(
                kinds_agree(node.kind(), &value),
                "reported kind {:?} disagrees with the parsed value",
                node.kind()
            );
        } else {
            // The only value an independent parser refuses while the span is still exact is a
            // JSON string spelling an unpaired surrogate, which this indexer also refuses to
            // decode rather than replace.
            prop_assert_eq!(node.kind(), JsonValueKind::String);
            let limits =
                generous_limits().map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert!(decode_json_string(request, span, limits).is_err());
        }
        if decodes_within_bound && node.kind() == JsonValueKind::String {
            if let Ok(expected) = serde_json::from_slice::<String>(slice) {
                let limits =
                    generous_limits().map_err(|error| TestCaseError::fail(error.to_string()))?;
                let decoded = decode_json_string(request, span, limits)
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                prop_assert_eq!(
                    decoded,
                    expected,
                    "a decoded string disagrees with serde_json"
                );
            }
        }
    }
    Ok(())
}

/// Asserts containment of every child and non-overlap of every sibling payload.
fn assert_structure_is_nested(index: &RawSpanIndex) -> Result<(), TestCaseError> {
    for node in index.nodes() {
        let children: Vec<&SpanNode> = index.children(node.id()).collect();
        prop_assert_eq!(
            u32::try_from(children.len()).unwrap_or(u32::MAX),
            node.child_count(),
            "the child list disagrees with the child count"
        );
        for (position, child) in children.iter().enumerate() {
            prop_assert_eq!(child.parent(), Some(node.id()));
            prop_assert!(
                node.span().covers(child.span()),
                "a child span escaped its parent"
            );
            prop_assert!(child.depth() > node.depth());
            if let Some(name) = child.name_span() {
                prop_assert!(node.span().covers(name));
                prop_assert!(
                    name.end() <= child.span().start(),
                    "a member name overlapped its own value"
                );
            }
            for other in children.iter().skip(position.saturating_add(1)) {
                prop_assert!(
                    !child.span().overlaps(other.span()),
                    "sibling payload spans overlapped"
                );
                prop_assert!(child.span().end() <= other.span().start());
                if let Some(name) = other.name_span() {
                    prop_assert!(!child.span().overlaps(name));
                }
            }
        }
        if node.kind().is_container() {
            continue;
        }
        prop_assert_eq!(node.child_count(), 0, "a scalar reported members");
    }
    Ok(())
}

/// Asserts that the root span is the whole document, whitespace excluded.
fn assert_root_is_the_whole_document(
    index: &RawSpanIndex,
    request: &[u8],
) -> Result<(), TestCaseError> {
    let Some(root) = index.root() else {
        return Err(TestCaseError::fail("no root value"));
    };
    let Some(slice) = root.span().slice(request) else {
        return Err(TestCaseError::fail("the root span could not be sliced"));
    };
    let (Ok(whole), Ok(root_value)) = (
        serde_json::from_slice::<Value>(request),
        serde_json::from_slice::<Value>(slice),
    ) else {
        return Ok(());
    };
    prop_assert_eq!(whole, root_value, "the root span is not the whole document");
    Ok(())
}

const fn is_json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

const fn kinds_agree(kind: JsonValueKind, value: &Value) -> bool {
    matches!(
        (kind, value),
        (JsonValueKind::Object, Value::Object(_))
            | (JsonValueKind::Array, Value::Array(_))
            | (JsonValueKind::String, Value::String(_))
            | (JsonValueKind::Number, Value::Number(_))
            | (JsonValueKind::Boolean, Value::Bool(_))
            | (JsonValueKind::Null, Value::Null)
    )
}

/// Generates one JSON document with arbitrary insignificant whitespace.
fn json_document() -> impl Strategy<Value = String> {
    (whitespace(), json_value(), whitespace())
        .prop_map(|(leading, value, trailing)| format!("{leading}{value}{trailing}"))
}

fn json_value() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("null".to_owned()),
        Just("true".to_owned()),
        Just("false".to_owned()),
        Just("0".to_owned()),
        Just("-1.5e-3".to_owned()),
        (-9999_i64..9999).prop_map(|value| value.to_string()),
        string_body().prop_map(|body| format!("\"{body}\"")),
    ];
    leaf.prop_recursive(4, 64, 4, |inner| {
        prop_oneof![
            (
                prop::collection::vec((whitespace(), inner.clone()), 0..4),
                whitespace()
            )
                .prop_map(|(elements, tail)| array_text(&elements, tail)),
            (
                prop::collection::vec((whitespace(), member_name(), whitespace(), inner), 0..4),
                whitespace()
            )
                .prop_map(|(members, tail)| object_text(&members, tail)),
        ]
    })
}

fn array_text(elements: &[(&'static str, String)], tail: &'static str) -> String {
    let mut text = String::from("[");
    for (position, (lead, element)) in elements.iter().enumerate() {
        if position > 0 {
            text.push(',');
        }
        text.push_str(lead);
        text.push_str(element);
    }
    text.push_str(tail);
    text.push(']');
    text
}

fn object_text(
    members: &[(&'static str, &'static str, &'static str, String)],
    tail: &'static str,
) -> String {
    let mut text = String::from("{");
    for (position, (lead, name, gap, value)) in members.iter().enumerate() {
        if position > 0 {
            text.push(',');
        }
        text.push_str(lead);
        text.push('"');
        text.push_str(name);
        text.push('"');
        text.push_str(gap);
        text.push(':');
        text.push_str(gap);
        text.push_str(value);
    }
    text.push_str(tail);
    text.push('}');
    text
}

fn whitespace() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["", " ", "\n", "\r\n", "\t", "  ", " \r\n\t "])
}

/// Member names, including names that collide only once decoded and names needing pointer escapes.
fn member_name() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "input",
        "instructions",
        "a",
        r"\u0061",
        "a/b",
        "c~d",
        "",
        "\u{1f600}",
    ])
}

/// String bodies covering every escape shape the span contract names.
fn string_body() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "",
        "hello",
        r#"say \"hi\""#,
        r"a\\b",
        r"a\/b",
        r"line\nbreak\ttab",
        r"\u00e9",
        r"\ud83d\ude00",
        "\u{1f600}",
        r"\u0000",
        "\u{4f60}\u{597d}",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ])
}
