#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_context::{ContextAnalysisStatus, RawSpanIndex};

fuzz_target!(|data: &[u8]| {
    let limits = common::limits();
    let first = RawSpanIndex::build(data, limits);
    let second = RawSpanIndex::build(data, limits);

    // The index retains positions and metadata only: the same bytes must produce the same result,
    // and constructing it must not mutate the fuzz input.
    assert_eq!(first.nodes(), second.nodes());
    assert_eq!(first.status(), second.status());
    assert_eq!(first.limit_reached(), second.limit_reached());
    assert_eq!(first.analyzed_bytes(), second.analyzed_bytes());
    assert_eq!(first.skipped_bytes(), second.skipped_bytes());
    assert_eq!(
        first.analyzed_bytes().saturating_add(first.skipped_bytes()),
        common::as_u64(data.len())
    );
    assert!(first.len() <= limits.max_blocks.get());
    assert_eq!(
        first.status() == ContextAnalysisStatus::ResourceLimit,
        first.limit_reached().is_some()
    );

    for node in first.nodes() {
        let span = node.span();
        assert!(span.start() <= span.end());
        assert!(span.end() <= common::as_u64(data.len()));
        if node.is_complete() {
            assert!(span.slice(data).is_some());
        }

        let children: Vec<_> = first.children(node.id()).collect();
        assert_eq!(children.len(), usize::try_from(node.child_count()).unwrap_or(usize::MAX));
        for (position, child) in children.iter().enumerate() {
            assert_eq!(child.parent(), Some(node.id()));
            assert!(span.covers(child.span()));
            assert!(child.depth() > node.depth());
            for other in children.iter().skip(position.saturating_add(1)) {
                assert!(!child.span().overlaps(other.span()));
                assert!(child.span().end() <= other.span().start());
            }
        }
    }
});
