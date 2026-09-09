//! Deferred policy boundary for Tracepress.

/// Compile-time marker for the deferred policy boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct PolicyBoundary;

impl PolicyBoundary {
    /// Creates the no-op Phase 0 boundary marker.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
