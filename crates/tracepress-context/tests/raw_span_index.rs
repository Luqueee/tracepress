//! Span contract of the bounded raw JSON indexer over the original request bytes.

use std::{error::Error, fmt::Write as _};

use tracepress_context::{
    AnalysisClock, ContextAnalysisLimitField, ContextAnalysisLimitValues, ContextAnalysisLimits,
    ContextAnalysisLimitsError, ContextAnalysisStatus, ContextDigest, JsonStringDecodeError,
    JsonValueKind, MonotonicClock, RawSpan, RawSpanIndex, SpanNode, decode_json_string,
};

include!("support/span_fixtures.rs");

type Outcome = Result<(), Box<dyn Error>>;

const CANARY_INSTRUCTIONS: &str = "be brief";
const CANARY_TEXT: &str = "hello";
/// A deterministic clock that counts every bounded-analysis checkpoint.
#[derive(Default)]
struct CountingClock {
    calls: std::cell::Cell<u64>,
}

impl CountingClock {
    fn calls(&self) -> u64 {
        self.calls.get()
    }
}

impl AnalysisClock for CountingClock {
    fn elapsed_ms(&self) -> u64 {
        self.calls.set(self.calls.get().saturating_add(1));
        0
    }
}

/// A clock that expires after a fixed number of deterministic checkpoints.
struct ExpiringClock {
    calls: std::cell::Cell<u64>,
    allowed_calls: u64,
}

impl ExpiringClock {
    const fn after(allowed_calls: u64) -> Self {
        Self {
            calls: std::cell::Cell::new(0),
            allowed_calls,
        }
    }
}

impl AnalysisClock for ExpiringClock {
    fn elapsed_ms(&self) -> u64 {
        let calls = self.calls.get().saturating_add(1);
        self.calls.set(calls);
        if calls > self.allowed_calls {
            ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS.saturating_add(1)
        } else {
            0
        }
    }
}

#[test]
fn whitespace_formatting_never_moves_a_span() -> Outcome {
    let documents = [
        (
            "minified",
            "{\"instructions\":\"be brief\",\"input\":\"hello\"}".to_owned(),
        ),
        (
            "pretty",
            "{\n  \"instructions\": \"be brief\",\n  \"input\": \"hello\"\n}\n".to_owned(),
        ),
        (
            "crlf",
            "{\r\n  \"instructions\": \"be brief\",\r\n  \"input\": \"hello\"\r\n}\r\n".to_owned(),
        ),
        (
            "tabs",
            "{\t\"instructions\":\t\"be brief\",\t\"input\":\t\"hello\"\t}".to_owned(),
        ),
    ];

    for (label, document) in documents {
        // Given
        let bytes = document.as_bytes();

        // When
        let index = index_document(&document)?;

        // Then
        assert_eq!(
            index.status(),
            ContextAnalysisStatus::Complete,
            "{label} must index completely"
        );
        let root = index.root().ok_or("no root value")?;
        assert_eq!(root.kind(), JsonValueKind::Object, "{label} root kind");
        assert_eq!(
            span_text(&document, root)?,
            document.trim(),
            "{label} root span must exclude surrounding whitespace"
        );
        assert_eq!(root.child_count(), 2, "{label} member count");
        let instructions = index
            .member(bytes, root.id(), "instructions")
            .ok_or("no instructions member")?;
        assert_eq!(
            span_text(&document, instructions)?,
            "\"be brief\"",
            "{label} instructions span must include its quotes"
        );
        let input = index
            .member(bytes, root.id(), "input")
            .ok_or("no input member")?;
        assert_eq!(
            span_text(&document, input)?,
            "\"hello\"",
            "{label} input span must include its quotes"
        );
        assert_eq!(
            decode_json_string(bytes, input.span(), generous_limits()?)?,
            CANARY_TEXT,
            "{label} decoded input"
        );
        assert_eq!(
            decode_json_string(bytes, instructions.span(), generous_limits()?)?,
            CANARY_INSTRUCTIONS,
            "{label} decoded instructions"
        );
    }
    Ok(())
}

#[test]
fn escaped_content_changes_the_decoded_value_never_the_span() -> Outcome {
    let cases = [
        ("empty string", "\"\"", String::new()),
        (
            "escaped quote",
            "\"say \\\"hi\\\"\"",
            "say \"hi\"".to_owned(),
        ),
        ("escaped backslash", "\"a\\\\b\"", "a\\b".to_owned()),
        ("escaped slash", "\"a\\/b\"", "a/b".to_owned()),
        ("escaped control", "\"a\\tb\\nc\"", "a\tb\nc".to_owned()),
        ("short escape", "\"\\u00e9\"", "\u{e9}".to_owned()),
        (
            "surrogate pair",
            "\"\\ud83d\\ude00\"",
            "\u{1f600}".to_owned(),
        ),
        ("emoji utf8", "\"\u{1f600}\"", "\u{1f600}".to_owned()),
        ("escaped nul", "\"\\u0000\"", "\u{0}".to_owned()),
        (
            "multi byte",
            "\"\u{4f60}\u{597d}\"",
            "\u{4f60}\u{597d}".to_owned(),
        ),
    ];

    for (label, literal, expected) in cases {
        // Given
        let document = format!("{{\"input\":{literal}}}");
        let bytes = document.as_bytes();

        // When
        let index = index_document(&document)?;

        // Then
        assert_eq!(
            index.status(),
            ContextAnalysisStatus::Complete,
            "{label} must index completely"
        );
        let root = index.root().ok_or("no root value")?;
        let input = index
            .member(bytes, root.id(), "input")
            .ok_or("no input member")?;
        assert_eq!(input.kind(), JsonValueKind::String, "{label} kind");
        assert_eq!(
            span_text(&document, input)?,
            literal,
            "{label} span must be the literal bytes"
        );
        assert_eq!(
            input.raw_bytes(),
            u64::try_from(literal.len())?,
            "{label} raw byte count"
        );
        assert_eq!(
            decode_json_string(bytes, input.span(), generous_limits()?)?,
            expected,
            "{label} decoded value"
        );
        let content = input.content_span().ok_or("no content span")?;
        assert_eq!(
            content.len_bytes(),
            u64::try_from(literal.len())?.saturating_sub(2),
            "{label} content span excludes both quotes"
        );
    }
    Ok(())
}

#[test]
fn scalar_spans_are_the_literal_text() -> Outcome {
    // Given
    let document = "{\"a\":-1.5e-3,\"b\":0,\"c\":true,\"d\":false,\"e\":null}";
    let bytes = document.as_bytes();

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    let root = index.root().ok_or("no root value")?;
    let expected = [
        ("a", "-1.5e-3", JsonValueKind::Number),
        ("b", "0", JsonValueKind::Number),
        ("c", "true", JsonValueKind::Boolean),
        ("d", "false", JsonValueKind::Boolean),
        ("e", "null", JsonValueKind::Null),
    ];
    for (name, text, kind) in expected {
        let node = index
            .member(bytes, root.id(), name)
            .ok_or("missing member")?;
        assert_eq!(span_text(document, node)?, text, "{name} span");
        assert_eq!(node.kind(), kind, "{name} kind");
        assert!(node.is_complete(), "{name} must be complete");
    }
    Ok(())
}

#[test]
fn empty_and_nested_containers_are_indexed_exactly() -> Outcome {
    let cases = [
        ("empty object", "{}", 1_usize, 0_u32),
        ("empty array", "[]", 1, 0),
        ("empty nested array", "{\"input\":[]}", 2, 1),
        ("nested arrays", "{\"input\":[[1,2],[3]]}", 7, 1),
        ("nested objects", "{\"a\":{\"b\":{\"c\":1}}}", 4, 1),
    ];

    for (label, document, nodes, root_children) in cases {
        // When
        let index = index_document(document)?;

        // Then
        assert_eq!(
            index.status(),
            ContextAnalysisStatus::Complete,
            "{label} status"
        );
        assert_eq!(index.len(), nodes, "{label} indexed value count");
        let root = index.root().ok_or("no root value")?;
        assert_eq!(root.child_count(), root_children, "{label} root children");
        assert_eq!(span_text(document, root)?, document, "{label} root span");
        assert!(
            index.nodes().iter().all(SpanNode::is_complete),
            "{label} every value must be complete"
        );
    }
    Ok(())
}

#[test]
fn nested_values_carry_pointer_paths_and_contained_spans() -> Outcome {
    // Given
    let document = "{\"input\":[{\"content\":[{\"text\":\"hello\"}]}]}";
    let bytes = document.as_bytes();

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    let root = index.root().ok_or("no root value")?;
    let input = index
        .member(bytes, root.id(), "input")
        .ok_or("no input member")?;
    assert_eq!(input.semantic_path().as_str(), "/input");
    let item = index.children(input.id()).next().ok_or("no first item")?;
    assert_eq!(item.semantic_path().as_str(), "/input/0");
    assert_eq!(item.array_index(), Some(0));
    let content = index
        .member(bytes, item.id(), "content")
        .ok_or("no content member")?;
    assert_eq!(content.semantic_path().as_str(), "/input/0/content");
    let part = index.children(content.id()).next().ok_or("no first part")?;
    let text = index
        .member(bytes, part.id(), "text")
        .ok_or("no text member")?;
    assert_eq!(text.semantic_path().as_str(), "/input/0/content/0/text");
    assert_eq!(span_text(document, text)?, "\"hello\"");
    assert!(
        input.span().covers(text.span()),
        "a child span must lie inside its parent"
    );
    assert_eq!(text.depth(), 6);
    Ok(())
}

#[test]
fn input_as_a_bare_string_and_as_an_item_array_are_both_indexed() -> Outcome {
    // Given
    let bare = "{\"input\":\"hello\"}";
    let items = "{\"input\":[{\"role\":\"user\",\"content\":\"hello\"}]}";

    // When
    let bare_index = index_document(bare)?;
    let items_index = index_document(items)?;

    // Then
    let bare_root = bare_index.root().ok_or("no root value")?;
    let bare_input = bare_index
        .member(bare.as_bytes(), bare_root.id(), "input")
        .ok_or("no input member")?;
    assert_eq!(bare_input.kind(), JsonValueKind::String);
    assert_eq!(span_text(bare, bare_input)?, "\"hello\"");

    let items_root = items_index.root().ok_or("no root value")?;
    let items_input = items_index
        .member(items.as_bytes(), items_root.id(), "input")
        .ok_or("no input member")?;
    assert_eq!(items_input.kind(), JsonValueKind::Array);
    assert_eq!(
        span_text(items, items_input)?,
        "[{\"role\":\"user\",\"content\":\"hello\"}]"
    );
    assert_eq!(items_input.child_count(), 1);
    Ok(())
}

#[test]
fn pointer_tokens_escape_slashes_and_tildes() -> Outcome {
    // Given
    let document = "{\"a/b\":{\"c~d\":1}}";
    let bytes = document.as_bytes();

    // When
    let index = index_document(document)?;

    // Then
    let root = index.root().ok_or("no root value")?;
    let outer = index
        .member(bytes, root.id(), "a/b")
        .ok_or("no slashed member")?;
    assert_eq!(outer.semantic_path().as_str(), "/a~1b");
    let inner = index
        .member(bytes, outer.id(), "c~d")
        .ok_or("no tilde member")?;
    assert_eq!(inner.semantic_path().as_str(), "/a~1b/c~0d");
    Ok(())
}

#[test]
fn duplicate_member_names_keep_every_occurrence() -> Outcome {
    // Given
    let document = "{\"input\":\"first\",\"other\":1,\"input\":\"second\"}";
    let bytes = document.as_bytes();

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(
        index.status(),
        ContextAnalysisStatus::Partial,
        "an ambiguous path degrades the analysis, never drops a member"
    );
    assert!(index.duplicate_key_detected());
    let root = index.root().ok_or("no root value")?;
    assert!(root.duplicate_key_detected());
    assert_eq!(root.child_count(), 3);
    let colliding: Vec<&SpanNode> = index
        .children(root.id())
        .filter(|node| node.name_equals(bytes, "input"))
        .collect();
    assert_eq!(colliding.len(), 2, "both colliding members stay indexed");
    let first = colliding.first().ok_or("no first collision")?;
    let second = colliding.get(1).ok_or("no second collision")?;
    assert_eq!(first.occurrence(), 0);
    assert_eq!(second.occurrence(), 1);
    assert_eq!(span_text(document, first)?, "\"first\"");
    assert_eq!(span_text(document, second)?, "\"second\"");
    assert_eq!(
        first.semantic_path().as_str(),
        second.semantic_path().as_str()
    );
    assert!(
        !first.span().overlaps(second.span()),
        "sibling payload spans must not overlap"
    );
    assert_eq!(
        index.member(bytes, root.id(), "input").map(SpanNode::id),
        Some(first.id()),
        "a name lookup resolves nothing on a caller's behalf"
    );
    Ok(())
}

#[test]
fn a_duplicate_member_name_spelled_with_an_escape_is_detected() -> Outcome {
    // Given
    let escaped = "{\"a\":1,\"\\u0061\":2}";
    let distinct = "{\"a\":1,\"b\":2}";

    // When
    let escaped_index = index_document(escaped)?;
    let distinct_index = index_document(distinct)?;

    // Then
    assert!(
        escaped_index.duplicate_key_detected(),
        "names are compared decoded, so an escaped spelling still collides"
    );
    assert_eq!(escaped_index.status(), ContextAnalysisStatus::Partial);
    let escaped_root = escaped_index.root().ok_or("no root value")?;
    let second = escaped_index
        .children(escaped_root.id())
        .nth(1)
        .ok_or("no second member")?;
    assert_eq!(second.occurrence(), 1);
    assert_eq!(span_text(escaped, second)?, "2");

    assert!(!distinct_index.duplicate_key_detected());
    assert_eq!(distinct_index.status(), ContextAnalysisStatus::Complete);
    Ok(())
}

#[test]
fn equal_member_names_in_sibling_objects_are_not_a_collision() -> Outcome {
    // Given
    let document = "[{\"a\":1},{\"a\":2}]";

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    assert!(
        !index.duplicate_key_detected(),
        "a name collides only with the names of its own object"
    );
    Ok(())
}

#[test]
fn unknown_members_contribute_their_own_spans() -> Outcome {
    // Given
    let document = "{\"model\":\"gpt\",\"mystery\":{\"deep\":[1,2]},\"input\":\"hi\"}";
    let bytes = document.as_bytes();

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(
        index.status(),
        ContextAnalysisStatus::Complete,
        "an unrecognised member is still just a value with a span"
    );
    assert_eq!(index.analyzed_bytes(), u64::try_from(document.len())?);
    assert_eq!(index.skipped_bytes(), 0);
    let root = index.root().ok_or("no root value")?;
    assert_eq!(root.child_count(), 3);
    let mystery = index
        .member(bytes, root.id(), "mystery")
        .ok_or("no mystery member")?;
    assert_eq!(span_text(document, mystery)?, "{\"deep\":[1,2]}");
    Ok(())
}

#[test]
fn a_large_string_keeps_an_exact_span_and_refuses_a_bounded_decode() -> Outcome {
    // Given
    let bound = usize::try_from(ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED)?;
    let oversized = "a".repeat(bound.saturating_add(1024));
    let document = format!("{{\"input\":\"{oversized}\"}}");
    let bytes = document.as_bytes();

    // When
    let index = index_document(&document)?;

    // Then
    assert_eq!(
        index.status(),
        ContextAnalysisStatus::Complete,
        "a long string is scanned, only its decoding is bounded"
    );
    let root = index.root().ok_or("no root value")?;
    let input = index
        .member(bytes, root.id(), "input")
        .ok_or("no input member")?;
    assert_eq!(
        input.raw_bytes(),
        u64::try_from(oversized.len().saturating_add(2))?
    );
    assert_eq!(
        decode_json_string(bytes, input.span(), generous_limits()?),
        Err(JsonStringDecodeError::AboveInspectionBound {
            maximum_bytes: ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED,
        }),
        "a truncated decode is refused rather than reported as a value"
    );

    // And a string inside the bound still decodes
    let inside = "b".repeat(bound.saturating_sub(1));
    let small = format!("{{\"input\":\"{inside}\"}}");
    let small_index = index_document(&small)?;
    let small_root = small_index.root().ok_or("no root value")?;
    let small_input = small_index
        .member(small.as_bytes(), small_root.id(), "input")
        .ok_or("no input member")?;
    assert_eq!(
        decode_json_string(small.as_bytes(), small_input.span(), generous_limits()?)?.len(),
        inside.len()
    );
    Ok(())
}

#[test]
fn distinct_long_member_names_keep_full_identity_and_paths() -> Outcome {
    // Given
    let bound = usize::try_from(ContextAnalysisLimits::MAX_STRING_BYTES_INSPECTED)?;
    let prefix = "a".repeat(bound.saturating_add(1));
    let document = format!("{{\"{prefix}x\":1,\"{prefix}y\":2}}");
    let bytes = document.as_bytes();

    // When
    let index = index_document(&document)?;

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    assert!(!index.duplicate_key_detected());
    let root = index.root().ok_or("no object root")?;
    let children: Vec<&SpanNode> = index.children(root.id()).collect();
    assert_eq!(children.len(), 2);
    let first = children.first().ok_or("missing first child")?;
    let second = children.get(1).ok_or("missing second child")?;
    assert_eq!(first.occurrence(), 0);
    assert_eq!(second.occurrence(), 0);
    assert!(first.semantic_path().is_truncated());
    assert!(second.semantic_path().is_truncated());
    assert_eq!(
        first.semantic_path().as_str(),
        second.semantic_path().as_str(),
        "the retained prefixes are intentionally bounded"
    );
    assert_ne!(
        first.semantic_path().full_value_hash(),
        second.semantic_path().full_value_hash(),
        "the incremental full-path digest preserves distinct long names"
    );
    let first_path = format!("/{prefix}x");
    let second_path = format!("/{prefix}y");
    assert_eq!(
        first.semantic_path().full_value_hash(),
        Some(ContextDigest::from_bytes(first_path.as_bytes()))
    );
    assert_eq!(
        second.semantic_path().full_value_hash(),
        Some(ContextDigest::from_bytes(second_path.as_bytes()))
    );
    assert!(
        first.name_equals(bytes, &format!("{prefix}x")),
        "the first long name remains exactly addressable"
    );
    assert!(
        second.name_equals(bytes, &format!("{prefix}y")),
        "the second long name remains exactly addressable"
    );
    Ok(())
}

#[test]
fn an_unpaired_surrogate_keeps_its_span_and_refuses_to_decode() -> Outcome {
    // Given
    let document = "{\"input\":\"\\ud800\"}";
    let bytes = document.as_bytes();

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    let root = index.root().ok_or("no root value")?;
    let input = index
        .member(bytes, root.id(), "input")
        .ok_or("no input member")?;
    assert_eq!(span_text(document, input)?, "\"\\ud800\"");
    assert_eq!(
        decode_json_string(bytes, input.span(), generous_limits()?),
        Err(JsonStringDecodeError::UnpairedSurrogate)
    );
    Ok(())
}

#[test]
fn a_decode_of_a_span_that_is_not_a_string_is_refused() -> Outcome {
    // Given
    let document = "{\"input\":123}";
    let bytes = document.as_bytes();
    let index = index_document(document)?;
    let root = index.root().ok_or("no root value")?;
    let input = index
        .member(bytes, root.id(), "input")
        .ok_or("no input member")?;

    // When
    let decoded = decode_json_string(bytes, input.span(), generous_limits()?);
    let outside = decode_json_string(
        bytes,
        RawSpan::new(0, 4096).ok_or("inverted span")?,
        generous_limits()?,
    );

    // Then
    assert_eq!(decoded, Err(JsonStringDecodeError::NotAJsonString));
    assert_eq!(outside, Err(JsonStringDecodeError::NotAJsonString));
    Ok(())
}

#[test]
fn nesting_beyond_the_depth_bound_keeps_everything_inside_it() -> Outcome {
    // Given
    let depth = 4_u64;
    let limits = limits_with(|values| values.max_json_depth = depth)?;
    let document = format!("{}1{}", "[".repeat(64), "]".repeat(64));

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::JsonDepth)
    );
    assert_eq!(
        index.len(),
        usize::try_from(depth)?,
        "every value inside the depth bound is preserved"
    );
    for (position, node) in index.nodes().iter().enumerate() {
        assert_eq!(node.kind(), JsonValueKind::Array);
        assert_eq!(node.span().start(), u64::try_from(position)?);
        assert!(
            !node.is_complete(),
            "a container the scan never closed is reported incomplete"
        );
    }
    Ok(())
}

#[test]
fn scalar_beyond_the_depth_bound_is_not_indexed() -> Outcome {
    // Given
    let document = "[[null]]";
    let limits = limits_with(|values| values.max_json_depth = 2)?;

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::JsonDepth)
    );
    assert_eq!(index.len(), 2);
    for node in index.nodes() {
        assert!(node.depth() <= 2);
        assert_eq!(node.kind(), JsonValueKind::Array);
        assert!(!node.is_complete());
        assert!(node.span().end() <= u64::try_from(document.len())?);
    }
    let root = index.root().ok_or("no root value")?;
    let child = index.children(root.id()).next().ok_or("no nested array")?;
    assert!(root.span().covers(child.span()));
    assert_eq!(child.parent(), Some(root.id()));
    Ok(())
}

#[test]
fn a_document_above_the_analyzed_byte_bound_keeps_what_fits() -> Outcome {
    // Given
    let document = "{\"instructions\":\"be brief\",\"input\":\"hello world\"}";
    let limits = limits_with(|values| values.max_analyzed_bytes = 30)?;

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalyzedBytes)
    );
    assert!(index.analyzed_bytes() <= 30);
    assert_eq!(
        index.analyzed_bytes().saturating_add(index.skipped_bytes()),
        u64::try_from(document.len())?,
        "skipped bytes account for every byte that was never analysed"
    );
    let instructions = index
        .nodes()
        .iter()
        .find(|node| node.name_equals(document.as_bytes(), "instructions"))
        .ok_or("the member inside the bound was discarded")?;
    assert_eq!(span_text(document, instructions)?, "\"be brief\"");
    assert!(instructions.is_complete());
    let root = index.root().ok_or("no root value")?;
    assert!(
        !root.is_complete(),
        "the root never closed inside the analysed window"
    );
    Ok(())
}

#[test]
fn a_multi_byte_character_cut_by_the_byte_bound_is_not_malformed() -> Outcome {
    // Given
    let document = "{\"a\":\"\u{1f600}\u{1f600}\"}";
    let limits = limits_with(|values| values.max_analyzed_bytes = 8)?;

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(
        index.status(),
        ContextAnalysisStatus::ResourceLimit,
        "a bound that cuts a character is a bound, not malformed input"
    );
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalyzedBytes)
    );
    assert!(index.skipped_bytes() > 0);
    Ok(())
}

#[test]
fn repeated_name_comparisons_consume_work_budget() -> Outcome {
    // Given
    let mut document = String::from("{");
    for key in 0_u32..8191 {
        if key > 0 {
            document.push(',');
        }
        document.push_str("\"same\":0");
    }
    document.push('}');
    let limits = limits_with(|values| values.max_analysis_work_units = 256)?;

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalysisWorkUnits)
    );
    assert!(
        index.len() < 8192,
        "a comparison budget must stop the old unmetered probe loop"
    );
    Ok(())
}

#[test]
fn the_block_bound_stops_indexing_and_keeps_indexed_values() -> Outcome {
    // Given
    let limits = limits_with(|values| values.max_blocks = 4)?;
    let document = "[1,2,3,4,5,6,7,8]";

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::Blocks)
    );
    assert_eq!(index.len(), 4);
    for node in index.nodes().iter().skip(1) {
        assert_eq!(node.kind(), JsonValueKind::Number);
        assert!(node.is_complete());
    }
    Ok(())
}

#[test]
fn distinct_object_names_use_linear_bounded_checkpoints() -> Outcome {
    // Given
    let mut document = String::from("{");
    for key in 0_u32..8191 {
        if key > 0 {
            document.push(',');
        }
        write!(document, "\"k{key}\":0")?;
    }
    document.push('}');
    let clock = CountingClock::default();

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), generous_limits()?, &clock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    assert_eq!(index.len(), 8192);
    assert!(!index.duplicate_key_detected());
    assert!(
        clock.calls() >= 3 * 8191 && clock.calls() <= 6 * 8191,
        "name hashing, path decoding, and publication must use a linear number of checkpoints; got {}",
        clock.calls()
    );
    Ok(())
}

#[test]
fn one_work_unit_never_publishes_more_than_one_value_quantum() -> Outcome {
    // Given
    let limits = limits_with(|values| values.max_analysis_work_units = 1)?;
    let document = format!("[{}]", vec!["0"; 127].join(","));

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalysisWorkUnits)
    );
    assert_eq!(
        index.len(),
        64,
        "the rejected second value quantum must never become observable"
    );
    Ok(())
}

#[test]
fn long_string_and_scalar_poll_the_clock_before_completion() -> Outcome {
    // Given
    let string_document = format!("\"{}\"", "a".repeat(2 * 1024 * 1024));
    let number_document = "1".repeat(2 * 1024 * 1024);

    // When
    let string_index = RawSpanIndex::build_with_clock(
        string_document.as_bytes(),
        generous_limits()?,
        &ExpiringClock::after(4),
    );
    let number_index = RawSpanIndex::build_with_clock(
        number_document.as_bytes(),
        generous_limits()?,
        &ExpiringClock::after(2),
    );

    // Then
    assert_eq!(
        string_index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalysisWallTimeMs)
    );
    let string = string_index
        .root()
        .ok_or("started string was not retained")?;
    assert_eq!(string.kind(), JsonValueKind::String);
    assert!(!string.is_complete());
    assert!(string.span().end() > string.span().start());
    assert!(string.span().end() < u64::try_from(string_document.len())?);
    assert_eq!(
        number_index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalysisWallTimeMs)
    );
    assert!(
        number_index.is_empty(),
        "an over-time scalar that was never published must stay excluded"
    );
    Ok(())
}

#[test]
fn values_closed_at_the_analyzed_byte_boundary_remain_complete() -> Outcome {
    // Given
    let object_document = "{} ";
    let array_document = "[\"x\",0]";
    let boolean_document = "true ";
    let null_document = "null ";
    let object_limits = limits_with(|values| values.max_analyzed_bytes = 2)?;
    let array_limits = limits_with(|values| values.max_analyzed_bytes = 4)?;
    let boolean_limits = limits_with(|values| values.max_analyzed_bytes = 4)?;
    let null_limits = limits_with(|values| values.max_analyzed_bytes = 4)?;

    // When
    let object =
        RawSpanIndex::build_with_clock(object_document.as_bytes(), object_limits, &FrozenClock);
    let array =
        RawSpanIndex::build_with_clock(array_document.as_bytes(), array_limits, &FrozenClock);
    let boolean =
        RawSpanIndex::build_with_clock(boolean_document.as_bytes(), boolean_limits, &FrozenClock);
    let null = RawSpanIndex::build_with_clock(null_document.as_bytes(), null_limits, &FrozenClock);

    // Then
    assert!(object.root().ok_or("no object root")?.is_complete());
    let array_root = array.root().ok_or("no array root")?;
    assert!(!array_root.is_complete());
    let child = array
        .children(array_root.id())
        .next()
        .ok_or("no string child")?;
    assert_eq!(
        child.span(),
        RawSpan::new(1, 4).ok_or("invalid expected span")?
    );
    assert!(child.is_complete());
    for scalar in [&boolean, &null] {
        assert_eq!(scalar.status(), ContextAnalysisStatus::ResourceLimit);
        assert_eq!(
            scalar.limit_reached(),
            Some(ContextAnalysisLimitField::AnalyzedBytes)
        );
        assert!(
            scalar.root().ok_or("no scalar root")?.is_complete(),
            "a scalar closed at the analysed boundary remains complete"
        );
    }
    Ok(())
}

#[test]
fn a_string_cut_inside_its_body_retains_its_observed_span() -> Outcome {
    // Given
    let document = "{\"input\":\"abcdef\"}";
    let limits = limits_with(|values| values.max_analyzed_bytes = 13)?;

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    let input = index
        .nodes()
        .iter()
        .find(|node| node.name_equals(document.as_bytes(), "input"))
        .ok_or("incomplete /input node was discarded")?;
    assert_eq!(input.kind(), JsonValueKind::String);
    assert_eq!(
        input.span(),
        RawSpan::new(9, 13).ok_or("invalid expected span")?
    );
    assert!(!input.is_complete());
    Ok(())
}

#[test]
fn the_work_unit_bound_stops_indexing_and_keeps_indexed_values() -> Outcome {
    // Given
    let limits = limits_with(|values| values.max_analysis_work_units = 1)?;
    let elements = (0..400).fold(String::from("["), |mut text, value| {
        if value > 0 {
            text.push(',');
        }
        text.push('7');
        text
    });
    let document = format!("{elements}]");

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalysisWorkUnits)
    );
    assert!(
        index.len() >= 64,
        "work already charged inside the bound stays indexed, found {}",
        index.len()
    );
    assert!(index.len() < 401, "the bound must actually stop the scan");
    Ok(())
}

#[test]
fn the_wall_time_bound_stops_indexing_and_keeps_indexed_values() -> Outcome {
    // Given
    struct StoppedClock;
    impl AnalysisClock for StoppedClock {
        fn elapsed_ms(&self) -> u64 {
            ContextAnalysisLimits::MAX_ANALYSIS_WALL_TIME_MS.saturating_mul(4)
        }
    }
    let document = "{\"input\":[1,2,3]}";

    // When
    let index =
        RawSpanIndex::build_with_clock(document.as_bytes(), generous_limits()?, &StoppedClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::ResourceLimit);
    assert_eq!(
        index.limit_reached(),
        Some(ContextAnalysisLimitField::AnalysisWallTimeMs)
    );
    assert!(
        index.is_empty(),
        "a value must not be published after its pre-publication clock check fails"
    );
    Ok(())
}

#[test]
fn truncated_json_is_malformed_and_keeps_what_was_indexed() -> Outcome {
    // Given
    let document = "{\"instructions\":\"be brief\",\"input\":\"hel";

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Malformed);
    assert_eq!(index.limit_reached(), None);
    let instructions = index
        .nodes()
        .iter()
        .find(|node| node.name_equals(document.as_bytes(), "instructions"))
        .ok_or("the member before the truncation was discarded")?;
    assert_eq!(span_text(document, instructions)?, "\"be brief\"");
    assert!(instructions.is_complete());
    let root = index.root().ok_or("no root value")?;
    assert!(!root.is_complete());
    Ok(())
}

#[test]
fn syntax_in_a_truncated_window_stays_malformed() -> Outcome {
    // Given
    let document = "x trailing bytes";
    let limits = limits_with(|values| values.max_analyzed_bytes = 1)?;

    // When
    let index = RawSpanIndex::build_with_clock(document.as_bytes(), limits, &FrozenClock);

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Malformed);
    assert_eq!(index.limit_reached(), None);
    Ok(())
}

#[test]
fn invalid_bytes_after_the_work_budget_are_not_prevalidated() -> Outcome {
    for (label, suffix) in [("invalid utf8", vec![0xff]), ("raw nul", vec![0])] {
        // Given
        let mut bytes = b"{\"a\":\"".to_vec();
        bytes.extend_from_slice("a".repeat(8192).as_bytes());
        bytes.extend_from_slice(&suffix);
        bytes.extend_from_slice(b"\"}");
        let limits = limits_with(|values| values.max_analysis_work_units = 1)?;

        // When
        let index = RawSpanIndex::build_with_clock(&bytes, limits, &FrozenClock);

        // Then
        assert_eq!(
            index.status(),
            ContextAnalysisStatus::ResourceLimit,
            "{label} beyond the work budget must not be found by a prepass"
        );
        assert_eq!(
            index.limit_reached(),
            Some(ContextAnalysisLimitField::AnalysisWorkUnits),
            "{label} must report the budget axis that stopped scanning"
        );
    }
    Ok(())
}

#[test]
fn invalid_utf8_and_an_embedded_nul_are_malformed() -> Outcome {
    let cases: [(&str, Vec<u8>); 3] = [
        ("invalid utf8", b"{\"a\":\"\xff\xfe\"}".to_vec()),
        ("raw nul", b"{\"a\":\"b\0c\"}".to_vec()),
        ("lone continuation", b"{\"a\":\"\x80\"}".to_vec()),
    ];

    for (label, bytes) in cases {
        // When
        let index = RawSpanIndex::build_with_clock(&bytes, generous_limits()?, &FrozenClock);

        // Then
        assert_eq!(
            index.status(),
            ContextAnalysisStatus::Malformed,
            "{label} must be malformed"
        );
        assert!(
            !index.is_empty(),
            "{label} must retain values indexed before the malformed byte"
        );
        assert!(
            index.analyzed_bytes() > 0,
            "{label} must charge and account bytes before the malformed byte"
        );
        assert_eq!(
            index.analyzed_bytes().saturating_add(index.skipped_bytes()),
            u64::try_from(bytes.len())?,
            "{label} accounts every byte as analysed or skipped"
        );
    }
    Ok(())
}

#[test]
fn malformed_documents_are_refused_without_a_panic() -> Outcome {
    let documents = [
        "",
        "   ",
        "{",
        "[",
        "}",
        "{}extra",
        "[1,]",
        "{\"a\":}",
        "{\"a\" 1}",
        "{\"a\":1,}",
        "{\"a\":1 \"b\":2}",
        "[1 2]",
        "01",
        "-",
        "1.",
        "1e",
        "1e+",
        ".5",
        "+1",
        "tru",
        "nul",
        "NaN",
        "'a'",
        "\"unterminated",
        "\"bad\\escape\"",
        "\"\\u12\"",
        "\"\\uzzzz\"",
        "{\"a\":\"b\"\t,}",
    ];

    for document in documents {
        // When
        let index = index_document(document)?;

        // Then
        assert_eq!(
            index.status(),
            ContextAnalysisStatus::Malformed,
            "{document:?} must be refused as malformed"
        );
    }
    Ok(())
}

#[test]
fn a_raw_control_character_inside_a_string_is_malformed() -> Outcome {
    // Given
    let document = "{\"a\":\"line\u{1}break\"}";

    // When
    let index = index_document(document)?;

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Malformed);
    Ok(())
}

#[test]
fn indexing_never_mutates_the_request() -> Outcome {
    // Given
    let request = "{\"input\":[{\"text\":\"hello\"},1,true,null]}"
        .as_bytes()
        .to_vec();
    let original = request.clone();

    // When
    let index = RawSpanIndex::build(&request, generous_limits()?);

    // Then
    assert_eq!(request, original, "analysis must not touch its input");
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    Ok(())
}

#[test]
fn a_monotonic_clock_indexes_a_realistic_request_completely() -> Outcome {
    // Given
    let document = "{\"model\":\"gpt\",\"instructions\":\"be brief\",\"input\":[{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hello\"}]}],\"tools\":[]}";

    // When
    let index = RawSpanIndex::build_with_clock(
        document.as_bytes(),
        generous_limits()?,
        &MonotonicClock::started_now(),
    );

    // Then
    assert_eq!(index.status(), ContextAnalysisStatus::Complete);
    assert_eq!(index.analyzed_bytes(), u64::try_from(document.len())?);
    Ok(())
}

#[test]
fn debug_output_carries_no_request_content() -> Outcome {
    // Given
    let document = "{\"supersecretkey\":\"topsecretvalue\"}";

    // When
    let index = index_document(document)?;
    let index_debug = format!("{index:?}");
    let node_debug = format!("{:?}", index.nodes());

    // Then
    for canary in ["supersecretkey", "topsecretvalue"] {
        assert!(!index_debug.contains(canary), "index debug leaked {canary}");
        assert!(!node_debug.contains(canary), "node debug leaked {canary}");
    }
    Ok(())
}

fn span_text<'document>(
    document: &'document str,
    node: &SpanNode,
) -> Result<&'document str, Box<dyn Error>> {
    let bytes = node
        .span()
        .slice(document.as_bytes())
        .ok_or("span lies outside the document")?;
    Ok(std::str::from_utf8(bytes)?)
}

fn limits_with(
    adjust: impl FnOnce(&mut ContextAnalysisLimitValues),
) -> Result<ContextAnalysisLimits, ContextAnalysisLimitsError> {
    let mut values = analysis_values();
    adjust(&mut values);
    ContextAnalysisLimits::new(values)
}
