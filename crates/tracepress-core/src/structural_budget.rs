use crate::{MaxCpuWorkUnits, MaxIpcQueueItems, MaxJsonItems, MaxJsonNesting, MaxProcessingTimeMs};

/// Structural JSON metadata supplied by a bounded detector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonShape {
    nesting: u64,
    items: u64,
}

impl JsonShape {
    /// Creates structural metadata without parsing provider-specific content.
    #[must_use]
    pub const fn new(nesting: u64, items: u64) -> Self {
        Self { nesting, items }
    }
}

/// The structural JSON dimension that exhausted inspection capacity.
#[allow(
    clippy::exhaustive_enums,
    reason = "JSON structure has exactly the independently bounded dimensions exposed here"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonBudgetAxis {
    /// Maximum container nesting.
    Nesting,
    /// Maximum object members and array elements.
    Items,
}

/// Whether structural JSON metadata permits bounded inspection.
#[allow(
    clippy::exhaustive_enums,
    reason = "metadata inspection is either permitted or explicitly bypassed"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonInspectionDecision {
    /// Structural metadata is within both configured limits.
    Inspect,
    /// Inspection is bypassed without parsing or changing opaque body bytes.
    Bypass {
        /// The structural dimension that exceeded its limit.
        axis: JsonBudgetAxis,
        /// The detector-supplied structural count.
        observed: u64,
        /// The configured maximum structural count.
        maximum: u64,
    },
}

/// Classifies bounded structural metadata without parsing provider-specific JSON.
#[must_use]
pub const fn classify_json_shape(
    shape: JsonShape,
    maximum_nesting: MaxJsonNesting,
    maximum_items: MaxJsonItems,
) -> JsonInspectionDecision {
    if shape.nesting > maximum_nesting.get() {
        JsonInspectionDecision::Bypass {
            axis: JsonBudgetAxis::Nesting,
            observed: shape.nesting,
            maximum: maximum_nesting.get(),
        }
    } else if shape.items > maximum_items.get() {
        JsonInspectionDecision::Bypass {
            axis: JsonBudgetAxis::Items,
            observed: shape.items,
            maximum: maximum_items.get(),
        }
    } else {
        JsonInspectionDecision::Inspect
    }
}

/// Deterministic capacity decision for a bounded IPC queue.
#[allow(
    clippy::exhaustive_enums,
    reason = "queue admission is either accepted or rejected"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueDecision {
    /// The incoming items fit in the bounded queue.
    WithinLimit {
        /// Queue items after admission.
        total_items: u64,
    },
    /// The incoming items would exceed or overflow queue capacity.
    Reject {
        /// Queue items before the attempted admission.
        current_items: u64,
        /// Items in the attempted admission.
        incoming_items: u64,
        /// Configured queue capacity.
        maximum_items: u64,
    },
}

/// Classifies queue admission with checked arithmetic.
#[must_use]
pub const fn classify_ipc_queue(
    current_items: u64,
    incoming_items: u64,
    maximum: MaxIpcQueueItems,
) -> QueueDecision {
    match current_items.checked_add(incoming_items) {
        Some(total_items) if total_items <= maximum.get() => {
            QueueDecision::WithinLimit { total_items }
        }
        Some(_) | None => QueueDecision::Reject {
            current_items,
            incoming_items,
            maximum_items: maximum.get(),
        },
    }
}

/// Deterministic work charged to one processing budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessingCharge {
    elapsed_ms: u64,
    work_units: u64,
}

impl ProcessingCharge {
    /// Creates a charge from externally measured counters without reading a clock.
    #[must_use]
    pub const fn new(elapsed_ms: u64, work_units: u64) -> Self {
        Self {
            elapsed_ms,
            work_units,
        }
    }
}

/// The processing dimension that exhausted its hard budget.
#[allow(
    clippy::exhaustive_enums,
    reason = "processing has exactly time and CPU/work dimensions"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessingBudgetAxis {
    /// Supplied processing-time milliseconds.
    Time,
    /// Supplied CPU or abstract work units.
    CpuWork,
}

/// Result of charging deterministic processing counters.
#[allow(
    clippy::exhaustive_enums,
    reason = "a charge either continues or exhausts one explicit hard budget"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessingBudgetDecision {
    /// Both counters remain within their hard limits.
    Continue {
        /// Total supplied processing-time milliseconds.
        elapsed_ms: u64,
        /// Total supplied CPU or abstract work units.
        work_units: u64,
    },
    /// One counter would exceed or overflow its hard limit.
    Exhausted {
        /// The exhausted budget dimension.
        axis: ProcessingBudgetAxis,
        /// Counter value before the attempted charge.
        consumed_before: u64,
        /// Counter units in the attempted charge.
        attempted: u64,
        /// Configured hard maximum.
        maximum: u64,
    },
}

/// Pure accounting state for processing time and CPU/work limits.
///
/// Mutable accounting state cannot be duplicated into divergent counters.
///
/// ```compile_fail
/// use tracepress_core::ProcessingBudget;
///
/// fn require_clone<T: Clone>() {}
/// require_clone::<ProcessingBudget>();
/// ```
///
/// ```compile_fail
/// use tracepress_core::ProcessingBudget;
///
/// fn require_copy<T: Copy>() {}
/// require_copy::<ProcessingBudget>();
/// ```
#[derive(Debug, Eq, PartialEq)]
pub struct ProcessingBudget {
    maximum_time: MaxProcessingTimeMs,
    maximum_work: MaxCpuWorkUnits,
    elapsed_ms: u64,
    work_units: u64,
}

impl ProcessingBudget {
    /// Creates an unused deterministic processing budget.
    #[must_use]
    pub const fn new(maximum_time: MaxProcessingTimeMs, maximum_work: MaxCpuWorkUnits) -> Self {
        Self {
            maximum_time,
            maximum_work,
            elapsed_ms: 0,
            work_units: 0,
        }
    }

    /// Applies one charge atomically, preserving counters when either limit would be exceeded.
    pub const fn charge(&mut self, charge: ProcessingCharge) -> ProcessingBudgetDecision {
        let Some(next_time) = self.elapsed_ms.checked_add(charge.elapsed_ms) else {
            return self.exhausted_time(charge.elapsed_ms);
        };
        if next_time > self.maximum_time.get() {
            return self.exhausted_time(charge.elapsed_ms);
        }
        let Some(next_work) = self.work_units.checked_add(charge.work_units) else {
            return self.exhausted_work(charge.work_units);
        };
        if next_work > self.maximum_work.get() {
            return self.exhausted_work(charge.work_units);
        }
        self.elapsed_ms = next_time;
        self.work_units = next_work;
        ProcessingBudgetDecision::Continue {
            elapsed_ms: next_time,
            work_units: next_work,
        }
    }

    const fn exhausted_time(&self, attempted: u64) -> ProcessingBudgetDecision {
        ProcessingBudgetDecision::Exhausted {
            axis: ProcessingBudgetAxis::Time,
            consumed_before: self.elapsed_ms,
            attempted,
            maximum: self.maximum_time.get(),
        }
    }

    const fn exhausted_work(&self, attempted: u64) -> ProcessingBudgetDecision {
        ProcessingBudgetDecision::Exhausted {
            axis: ProcessingBudgetAxis::CpuWork,
            consumed_before: self.work_units,
            attempted,
            maximum: self.maximum_work.get(),
        }
    }
}
