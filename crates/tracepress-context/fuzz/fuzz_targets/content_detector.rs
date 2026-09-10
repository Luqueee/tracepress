#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use tracepress_context::{
    BlockContentMetadata, ContextBlockKind, DetectionConfidence, DetectedContentKind,
    STRUCTURAL_DETECTOR_VERSION, ShadowContentDetector, StructuralContentDetector,
};

fuzz_target!(|data: &[u8]| {
    let limits = common::limits();
    let detector = StructuralContentDetector::new(&limits);
    let observed = common::observed_prefix(data);
    let metadata = BlockContentMetadata::new(ContextBlockKind::ToolResult)
        .with_content_bytes(common::as_u64(data.len()));
    let first = detector.detect(observed, metadata);
    let second = detector.detect(observed, metadata);

    assert_eq!(first, second);
    assert_eq!(first.detector_version, STRUCTURAL_DETECTOR_VERSION);
    assert_eq!(detector.inspected_bytes_budget(), limits.max_string_bytes_inspected.get());
    if first.confidence == DetectionConfidence::Low {
        assert_eq!(first.kind, DetectedContentKind::Unknown);
    } else {
        assert_ne!(first.kind, DetectedContentKind::Unknown);
    }

    // References are envelopes, not content: the detector must abstain without resolving them.
    let reference = detector.detect(
        observed,
        BlockContentMetadata::new(ContextBlockKind::FileReference)
            .with_content_bytes(common::as_u64(data.len())),
    );
    assert_eq!(reference.kind, DetectedContentKind::Unknown);
    assert_eq!(reference.confidence, DetectionConfidence::Low);
    assert_eq!(reference.detector_version, STRUCTURAL_DETECTOR_VERSION);
});
