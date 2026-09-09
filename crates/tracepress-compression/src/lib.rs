//! Deferred compression boundary for Tracepress.

/// Compile-time marker for the deferred compression boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompressionBoundary;

impl CompressionBoundary {
    /// Creates the no-op Phase 0 boundary marker.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
