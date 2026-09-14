//! Bounded, deterministic `ToolResult` reduction primitives for Phase 4.3 shadow evaluation.
//!
//! Reducers deliberately keep the reduced view and the original bytes in transient memory only.
//! The caller may place the original in a local recovery store, but this module never persists or
//! serializes either representation. Unknown and non-ToolResult content is never eligible.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{BlockMetadata, CompressionLimits, TransformError, digest};

/// Version of the Phase 4.3 reducer contract.
pub const TOOL_AWARE_REDUCTION_VERSION: u32 = 1;

/// Conservative decision made before a reducer is allowed to produce a shadow view.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum ReductionPolicyDecision {
    /// Keep the complete `ToolResult` and do not produce a reduced view.
    KeepFull,
    /// Produce a reduced view in shadow mode; active use still requires quality gates.
    Reduce,
    /// Keep the full view while recording that a reducer-shaped candidate exists.
    KeepFullWithCandidate,
    /// The block is outside the reducer's explicit target.
    Unsupported,
}

/// Terminal state of one lossy reduction attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum ReductionStatus {
    /// A smaller reduced view was produced.
    Applicable,
    /// The structure is outside the reducer's target.
    NotApplicable,
    /// The reducer was applicable, but did not produce a smaller view.
    NoImprovement,
    /// A configured resource bound stopped the reducer.
    ResourceLimit,
    /// The input was not valid for the declared content kind.
    InvalidInput,
    /// An internal invariant failed.
    InternalError,
}

/// Metadata safe for persistence and Observatory transport.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReductionMetrics {
    /// Stable reducer identity.
    pub reducer_id: &'static str,
    /// Reducer implementation version.
    pub reducer_version: u32,
    /// Terminal reduction state.
    pub status: ReductionStatus,
    /// Original bytes inspected.
    pub input_bytes: u64,
    /// Reduced bytes visible to the model, when a view exists.
    pub visible_bytes: Option<u64>,
    /// Positive byte reduction, when the reduced view is smaller.
    pub gross_bytes_delta: Option<u64>,
    /// Existing input token estimate.
    pub input_estimated_tokens: Option<u64>,
    /// Local estimate for the reduced view.
    pub visible_estimated_tokens: Option<u64>,
    /// Positive estimated-token reduction, when comparable.
    pub gross_estimated_token_delta: Option<u64>,
    /// Monotonic processing time for this reducer invocation.
    pub processing_us: u64,
    /// Number of object fields omitted by the reducer.
    pub omitted_fields: u64,
    /// Number of array items omitted by the reducer.
    pub omitted_items: u64,
    /// Whether an original is available through a local recovery boundary.
    pub recovery_available: bool,
    /// Whether recovery returned bytes with the original fingerprint.
    pub recovery_verified: bool,
    /// Whether repeated evaluation produced the same visible fingerprint.
    pub deterministic: bool,
    /// Opaque local recovery identifier, never a path or raw content.
    pub recovery_id: Option<String>,
    /// SHA-256 fingerprint retained as a technical identifier, never raw content.
    pub original_fingerprint: [u8; 32],
    /// Fingerprint of the visible view, when one exists.
    pub visible_fingerprint: Option<[u8; 32]>,
    /// First byte at which the visible view differs from the original.
    pub first_modified_offset: Option<u64>,
    /// Number of common prefix bytes between original and visible view.
    pub preserved_prefix_bytes: Option<u64>,
    /// Whether the reduced representation remains ordinary provider-readable JSON.
    pub provider_readability: &'static str,
}

/// Transient lossy reduction result.
pub struct ReductionCandidate {
    metrics: ReductionMetrics,
    visible: Option<Box<[u8]>>,
    original: Option<Box<[u8]>>,
}

impl std::fmt::Debug for ReductionCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReductionCandidate")
            .field("metrics", &self.metrics)
            .field(
                "visible_bytes",
                &self.visible.as_ref().map(|value| value.len()),
            )
            .field(
                "original_bytes",
                &self.original.as_ref().map(|value| value.len()),
            )
            .finish()
    }
}

impl ReductionCandidate {
    /// Returns metadata safe for persistence.
    #[must_use]
    pub const fn metrics(&self) -> &ReductionMetrics {
        &self.metrics
    }

    /// Returns the transient reduced view for a shadow caller.
    #[must_use]
    pub fn visible(&self) -> Option<&[u8]> {
        self.visible.as_deref()
    }

    /// Recovers the original bytes through the local, in-memory recovery boundary.
    ///
    /// The original is intentionally unavailable when no reduced view was accepted.
    #[must_use]
    pub fn recover(&self) -> Option<Box<[u8]>> {
        self.original.clone()
    }

    /// Attaches the caller's existing token estimates without serializing candidate bytes.
    #[must_use]
    pub fn with_estimates(mut self, input: Option<u64>, visible: Option<u64>) -> Self {
        self.metrics.input_estimated_tokens = input;
        self.metrics.visible_estimated_tokens = visible;
        self.metrics.gross_estimated_token_delta = input
            .zip(visible)
            .and_then(|(before, after)| (after < before).then_some(before.saturating_sub(after)));
        self
    }

    /// Records the result of an independent repeated evaluation.
    #[must_use]
    pub const fn with_deterministic(mut self, deterministic: bool) -> Self {
        self.metrics.deterministic = deterministic;
        if !deterministic {
            self.metrics.status = ReductionStatus::InternalError;
        }
        self
    }
}

/// Reducer contract for deterministic, bounded `ToolResult` views.
pub trait ToolResultReducer: Send + Sync {
    /// Stable reducer identifier.
    fn id(&self) -> &'static str;

    /// Reducer implementation version.
    fn version(&self) -> u32;

    /// Returns whether the metadata is inside the reducer's explicit target.
    fn supports(&self, metadata: BlockMetadata) -> bool;

    /// Chooses a conservative policy before running the reducer.
    fn policy(&self, metadata: BlockMetadata, input_bytes: u64) -> ReductionPolicyDecision;

    /// Produces a transient reduced view and local recovery boundary.
    fn reduce(&self, input: &[u8], limits: &CompressionLimits) -> ReductionCandidate;
}

/// L1 shadow reducer: removes only structurally empty JSON object fields.
///
/// This is intentionally a shadow-only candidate. Empty values can be semantically meaningful to
/// a tool consumer, so no active request may use this reducer before objective quality evaluation.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonEmptyNoiseFieldReducer;

impl ToolResultReducer for JsonEmptyNoiseFieldReducer {
    fn id(&self) -> &'static str {
        "json.empty_noise_fields"
    }

    fn version(&self) -> u32 {
        TOOL_AWARE_REDUCTION_VERSION
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn policy(&self, metadata: BlockMetadata, input_bytes: u64) -> ReductionPolicyDecision {
        if !self.supports(metadata) {
            return ReductionPolicyDecision::Unsupported;
        }
        if input_bytes < 128 {
            ReductionPolicyDecision::KeepFull
        } else {
            ReductionPolicyDecision::KeepFullWithCandidate
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the bounded reducer keeps parsing, transformation, and metadata accounting together"
    )]
    fn reduce(&self, input: &[u8], limits: &CompressionLimits) -> ReductionCandidate {
        let started = std::time::Instant::now();
        let input_bytes = u64::try_from(input.len()).unwrap_or(u64::MAX);
        let original_fingerprint = digest(input);
        let base = |status| ReductionCandidate {
            metrics: ReductionMetrics {
                reducer_id: self.id(),
                reducer_version: self.version(),
                status,
                input_bytes,
                visible_bytes: None,
                gross_bytes_delta: None,
                input_estimated_tokens: None,
                visible_estimated_tokens: None,
                gross_estimated_token_delta: None,
                processing_us: elapsed_us(started),
                omitted_fields: 0,
                omitted_items: 0,
                recovery_available: false,
                recovery_verified: false,
                deterministic: true,
                recovery_id: None,
                original_fingerprint,
                visible_fingerprint: None,
                first_modified_offset: None,
                preserved_prefix_bytes: None,
                provider_readability: "human_readable_structured",
            },
            visible: None,
            original: None,
        };
        if input_bytes > limits.max_candidate_input_bytes
            || input_bytes > limits.max_shadow_memory_bytes
        {
            return base(ReductionStatus::ResourceLimit);
        }
        let Ok(value) = serde_json::from_slice::<Value>(input) else {
            return base(ReductionStatus::InvalidInput);
        };
        let mut work = 0_u64;
        let mut omitted_fields = 0_u64;
        let Ok(reduced_value) = strip_empty_fields(
            value,
            &mut work,
            limits.max_shadow_work_units,
            &mut omitted_fields,
        ) else {
            return base(ReductionStatus::ResourceLimit);
        };
        let Ok(visible) = serde_json::to_vec(&reduced_value) else {
            return base(ReductionStatus::InternalError);
        };
        let visible_bytes = u64::try_from(visible.len()).unwrap_or(u64::MAX);
        if visible_bytes > limits.max_candidate_output_bytes
            || visible_bytes.saturating_add(input_bytes) > limits.max_shadow_memory_bytes
            || elapsed_us(started) > limits.max_shadow_wall_time_ms.get().saturating_mul(1_000)
        {
            return base(ReductionStatus::ResourceLimit);
        }
        if visible_bytes >= input_bytes {
            return base(ReductionStatus::NoImprovement);
        }
        let gross_bytes_delta = input_bytes.saturating_sub(visible_bytes);
        let visible_fingerprint = digest(&visible);
        let preserved_prefix_bytes = common_prefix(input, &visible);
        let recovered = input.to_vec().into_boxed_slice();
        ReductionCandidate {
            metrics: ReductionMetrics {
                reducer_id: self.id(),
                reducer_version: self.version(),
                status: ReductionStatus::Applicable,
                input_bytes,
                visible_bytes: Some(visible_bytes),
                gross_bytes_delta: Some(gross_bytes_delta),
                input_estimated_tokens: None,
                visible_estimated_tokens: None,
                gross_estimated_token_delta: None,
                processing_us: elapsed_us(started),
                omitted_fields,
                omitted_items: 0,
                recovery_available: true,
                recovery_verified: digest(&recovered) == original_fingerprint,
                deterministic: true,
                recovery_id: Some(opaque_recovery_id(original_fingerprint)),
                original_fingerprint,
                visible_fingerprint: Some(visible_fingerprint),
                first_modified_offset: (preserved_prefix_bytes < input.len()
                    || preserved_prefix_bytes < visible.len())
                .then_some(u64::try_from(preserved_prefix_bytes).unwrap_or(u64::MAX)),
                preserved_prefix_bytes: Some(
                    u64::try_from(preserved_prefix_bytes).unwrap_or(u64::MAX),
                ),
                provider_readability: "human_readable_structured",
            },
            visible: Some(visible.into_boxed_slice()),
            original: Some(recovered),
        }
    }
}

/// L2 shadow reducer: moves exact constant primitive fields out of homogeneous JSON rows.
///
/// The visible representation remains ordinary JSON with an explicit `common` object and a
/// `rows` array. It is therefore readable by a provider without a Tracepress decoder, while the
/// original bytes remain available only to the local recovery boundary. This reducer is never
/// active by itself; objective quality evaluation is required before any request rewrite.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonRepeatedValueReducer;

impl ToolResultReducer for JsonRepeatedValueReducer {
    fn id(&self) -> &'static str {
        "json.repeated_value_elision"
    }

    fn version(&self) -> u32 {
        TOOL_AWARE_REDUCTION_VERSION
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn policy(&self, metadata: BlockMetadata, input_bytes: u64) -> ReductionPolicyDecision {
        if !self.supports(metadata) {
            return ReductionPolicyDecision::Unsupported;
        }
        if input_bytes < 128 {
            ReductionPolicyDecision::KeepFull
        } else {
            ReductionPolicyDecision::KeepFullWithCandidate
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the bounded reducer keeps parsing, transformation, and metadata accounting together"
    )]
    fn reduce(&self, input: &[u8], limits: &CompressionLimits) -> ReductionCandidate {
        let started = std::time::Instant::now();
        let input_bytes = u64::try_from(input.len()).unwrap_or(u64::MAX);
        let original_fingerprint = digest(input);
        let base = |status| ReductionCandidate {
            metrics: ReductionMetrics {
                reducer_id: self.id(),
                reducer_version: self.version(),
                status,
                input_bytes,
                visible_bytes: None,
                gross_bytes_delta: None,
                input_estimated_tokens: None,
                visible_estimated_tokens: None,
                gross_estimated_token_delta: None,
                processing_us: elapsed_us(started),
                omitted_fields: 0,
                omitted_items: 0,
                recovery_available: false,
                recovery_verified: false,
                deterministic: true,
                recovery_id: None,
                original_fingerprint,
                visible_fingerprint: None,
                first_modified_offset: None,
                preserved_prefix_bytes: None,
                provider_readability: "human_readable_structured",
            },
            visible: None,
            original: None,
        };

        if input_bytes > limits.max_candidate_input_bytes
            || input_bytes > limits.max_shadow_memory_bytes
        {
            return base(ReductionStatus::ResourceLimit);
        }

        // Reuse the strict JSON validator used by the lossless compressors. In particular, this
        // rejects duplicate object keys instead of silently changing their consumer semantics.
        match crate::json::validate_json(input, limits) {
            Ok(()) => {}
            Err(TransformError::NotApplicable) => return base(ReductionStatus::NotApplicable),
            Err(TransformError::ResourceLimit) => return base(ReductionStatus::ResourceLimit),
            Err(TransformError::InvalidInput) => return base(ReductionStatus::InvalidInput),
            Err(_) => return base(ReductionStatus::InternalError),
        }
        let Ok(Value::Array(rows)) = serde_json::from_slice::<Value>(input) else {
            return base(ReductionStatus::NotApplicable);
        };
        if rows.len() < 2 {
            return base(ReductionStatus::NotApplicable);
        }

        let mut work = 0_u64;
        let first = match rows.first().and_then(Value::as_object) {
            Some(first) if !first.is_empty() => first,
            _ => return base(ReductionStatus::NotApplicable),
        };
        let first_keys: BTreeSet<&str> = first.keys().map(String::as_str).collect();
        let mut common = Map::new();
        for key in &first_keys {
            work = work.saturating_add(rows.len() as u64);
            if work > limits.max_shadow_work_units {
                return base(ReductionStatus::ResourceLimit);
            }
            let Some(value) = first.get(*key) else {
                return base(ReductionStatus::NotApplicable);
            };
            if !is_json_scalar(value)
                || rows.iter().any(|row| {
                    let Some(object) = row.as_object() else {
                        return true;
                    };
                    let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
                    keys != first_keys || object.get(*key) != Some(value)
                })
            {
                continue;
            }
            let _previous = common.insert((*key).to_owned(), value.clone());
        }
        if common.is_empty() {
            return base(ReductionStatus::NotApplicable);
        }

        let mut reduced_rows = Vec::with_capacity(rows.len());
        let mut omitted_fields = 0_u64;
        for row in rows {
            let Some(mut object) = row.as_object().cloned() else {
                return base(ReductionStatus::NotApplicable);
            };
            for key in common.keys() {
                if object.remove(key).is_some() {
                    omitted_fields = omitted_fields.saturating_add(1);
                }
            }
            reduced_rows.push(Value::Object(object));
            work = work.saturating_add(1);
            if work > limits.max_shadow_work_units {
                return base(ReductionStatus::ResourceLimit);
            }
        }
        let mut reduced = Map::new();
        let _previous = reduced.insert("common".to_owned(), Value::Object(common));
        let _previous = reduced.insert("rows".to_owned(), Value::Array(reduced_rows));
        let Ok(visible) = serde_json::to_vec(&Value::Object(reduced)) else {
            return base(ReductionStatus::InternalError);
        };
        let visible_bytes = u64::try_from(visible.len()).unwrap_or(u64::MAX);
        if visible_bytes > limits.max_candidate_output_bytes
            || visible_bytes.saturating_add(input_bytes) > limits.max_shadow_memory_bytes
            || elapsed_us(started) > limits.max_shadow_wall_time_ms.get().saturating_mul(1_000)
        {
            return base(ReductionStatus::ResourceLimit);
        }
        if visible_bytes >= input_bytes {
            return base(ReductionStatus::NoImprovement);
        }

        let visible_fingerprint = digest(&visible);
        let preserved_prefix_bytes = common_prefix(input, &visible);
        let recovered = input.to_vec().into_boxed_slice();
        ReductionCandidate {
            metrics: ReductionMetrics {
                reducer_id: self.id(),
                reducer_version: self.version(),
                status: ReductionStatus::Applicable,
                input_bytes,
                visible_bytes: Some(visible_bytes),
                gross_bytes_delta: Some(input_bytes.saturating_sub(visible_bytes)),
                input_estimated_tokens: None,
                visible_estimated_tokens: None,
                gross_estimated_token_delta: None,
                processing_us: elapsed_us(started),
                omitted_fields,
                omitted_items: 0,
                recovery_available: true,
                recovery_verified: digest(&recovered) == original_fingerprint,
                deterministic: true,
                recovery_id: Some(opaque_recovery_id(original_fingerprint)),
                original_fingerprint,
                visible_fingerprint: Some(visible_fingerprint),
                first_modified_offset: (preserved_prefix_bytes < input.len()
                    || preserved_prefix_bytes < visible.len())
                .then_some(u64::try_from(preserved_prefix_bytes).unwrap_or(u64::MAX)),
                preserved_prefix_bytes: Some(
                    u64::try_from(preserved_prefix_bytes).unwrap_or(u64::MAX),
                ),
                provider_readability: "human_readable_structured",
            },
            visible: Some(visible.into_boxed_slice()),
            original: Some(recovered),
        }
    }
}

const fn is_json_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn elapsed_us(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

fn opaque_recovery_id(fingerprint: [u8; 32]) -> String {
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&fingerprint[..8]);
    format!("recovery-{:016x}", u64::from_be_bytes(prefix))
}

fn strip_empty_fields(
    value: Value,
    work: &mut u64,
    max_work: u64,
    omitted_fields: &mut u64,
) -> Result<Value, TransformError> {
    *work = work.saturating_add(1);
    if *work > max_work {
        return Err(TransformError::ResourceLimit);
    }
    match value {
        Value::Object(object) => {
            let mut retained = Map::new();
            for (key, child) in object {
                let empty = matches!(child, Value::Null)
                    || child.as_str().is_some_and(str::is_empty)
                    || child.as_array().is_some_and(Vec::is_empty)
                    || child.as_object().is_some_and(Map::is_empty);
                if empty {
                    *omitted_fields = omitted_fields.saturating_add(1);
                    continue;
                }
                let _previous = retained.insert(
                    key,
                    strip_empty_fields(child, work, max_work, omitted_fields)?,
                );
            }
            Ok(Value::Object(retained))
        }
        Value::Array(values) => values
            .into_iter()
            .map(|child| strip_empty_fields(child, work, max_work, omitted_fields))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockKind, BlockOrigin, DetectedKind};

    fn metadata() -> BlockMetadata {
        BlockMetadata::new(
            BlockOrigin::ToolGenerated,
            BlockKind::ToolResult,
            DetectedKind::Json,
            Some(100),
            500,
            2_000,
            Some(2),
            Some(true),
        )
    }

    #[test]
    fn removes_only_structurally_empty_object_fields_and_recovers_exactly() {
        let input = br#"{"name":"a","null_value":null,"empty":"","empty_array":[],"nested":{"keep":1,"drop":{}}}"#;
        let reducer = JsonEmptyNoiseFieldReducer;
        let result = reducer.reduce(input, &CompressionLimits::default());

        assert_eq!(result.metrics().status, ReductionStatus::Applicable);
        assert_eq!(result.metrics().omitted_fields, 4);
        assert!(result.metrics().recovery_available);
        assert!(result.metrics().recovery_verified);
        assert_eq!(result.recover().as_deref(), Some(input.as_slice()));
        assert!(result.visible().is_some());
        let visible = result.visible().unwrap_or_default();
        let parsed_result: Result<Value, _> = serde_json::from_slice(visible);
        assert!(parsed_result.is_ok());
        let parsed = parsed_result.unwrap_or(Value::Null);
        assert_eq!(parsed.get("name"), Some(&Value::String("a".to_owned())));
        assert!(parsed.get("null_value").is_none());
        assert!(
            parsed
                .get("nested")
                .and_then(Value::as_object)
                .is_some_and(|nested| nested.get("drop").is_none())
        );
    }

    #[test]
    fn reducer_is_deterministic_and_recovery_ids_are_input_scoped() {
        let reducer = JsonEmptyNoiseFieldReducer;
        let first = reducer.reduce(
            br#"{"keep":"value","drop":null}"#,
            &CompressionLimits::default(),
        );
        let second = reducer.reduce(
            br#"{"keep":"value","drop":null}"#,
            &CompressionLimits::default(),
        );
        let other = reducer.reduce(
            br#"{"keep":"other","drop":null}"#,
            &CompressionLimits::default(),
        );

        assert_eq!(
            first.metrics().visible_fingerprint,
            second.metrics().visible_fingerprint
        );
        assert_eq!(first.metrics().recovery_id, second.metrics().recovery_id);
        assert_eq!(first.metrics().status, second.metrics().status);
        assert_eq!(first.visible(), second.visible());
        assert_ne!(first.metrics().recovery_id, other.metrics().recovery_id);
    }

    #[test]
    fn unknown_and_non_tool_results_are_unsupported() {
        let reducer = JsonEmptyNoiseFieldReducer;
        let mut unknown = metadata();
        unknown.detected_kind = DetectedKind::Unknown;
        let mut human = metadata();
        human.origin = BlockOrigin::HumanAuthored;

        assert!(!reducer.supports(unknown));
        assert_eq!(
            reducer.policy(unknown, 512),
            ReductionPolicyDecision::Unsupported
        );
        assert!(!reducer.supports(human));
    }

    #[test]
    fn work_and_input_limits_fail_closed() {
        let reducer = JsonEmptyNoiseFieldReducer;
        let limits = CompressionLimits {
            max_shadow_work_units: 1,
            ..CompressionLimits::default()
        };
        assert_eq!(
            reducer
                .reduce(br#"{"keep":{"nested":true},"drop":null}"#, &limits)
                .metrics()
                .status,
            ReductionStatus::ResourceLimit
        );

        let limits = CompressionLimits {
            max_candidate_input_bytes: 4,
            ..CompressionLimits::default()
        };
        assert_eq!(
            reducer
                .reduce(br#"{"drop":null}"#, &limits)
                .metrics()
                .status,
            ReductionStatus::ResourceLimit
        );
    }

    #[test]
    fn no_improvement_is_not_reported_as_zero_output() {
        let reducer = JsonEmptyNoiseFieldReducer;
        let result = reducer.reduce(br#"{"keep":"value"}"#, &CompressionLimits::default());
        assert_eq!(result.metrics().status, ReductionStatus::NoImprovement);
        assert!(result.metrics().visible_bytes.is_none());
        assert!(result.metrics().recovery_id.is_none());
    }

    #[test]
    fn repeated_values_are_elided_from_homogeneous_rows_and_recover_exactly() {
        let input = br#"[{"name":"alpha","status":"success","source":"tool-output","kind":"diagnostic"},{"name":"beta","status":"success","source":"tool-output","kind":"diagnostic"},{"name":"gamma","status":"success","source":"tool-output","kind":"diagnostic"},{"name":"delta","status":"success","source":"tool-output","kind":"diagnostic"}]"#;
        let reducer = JsonRepeatedValueReducer;
        let result = reducer.reduce(input, &CompressionLimits::default());

        assert_eq!(result.metrics().status, ReductionStatus::Applicable);
        assert_eq!(result.metrics().omitted_fields, 12);
        assert_eq!(result.recover().as_deref(), Some(input.as_slice()));
        assert!(result.metrics().recovery_verified);
        let visible: Value =
            serde_json::from_slice(result.visible().unwrap_or_default()).unwrap_or(Value::Null);
        let common = visible.get("common").and_then(Value::as_object);
        assert_eq!(
            common.and_then(|object| object.get("status")),
            Some(&Value::String("success".to_owned()))
        );
        assert_eq!(
            common.and_then(|object| object.get("source")),
            Some(&Value::String("tool-output".to_owned()))
        );
        let rows = visible.get("rows").and_then(Value::as_array);
        assert_eq!(rows.map(Vec::len), Some(4));
        assert!(
            rows.and_then(|rows| rows.first())
                .and_then(Value::as_object)
                .is_some_and(|object| object.get("status").is_none())
        );
    }

    #[test]
    fn repeated_value_reducer_rejects_duplicate_keys() {
        let reducer = JsonRepeatedValueReducer;
        let result = reducer.reduce(
            br#"[{"name":"a","status":"ok","status":"bad"},{"name":"b","status":"ok"}]"#,
            &CompressionLimits::default(),
        );
        assert_eq!(result.metrics().status, ReductionStatus::NotApplicable);
    }

    #[test]
    fn repeated_value_reducer_stays_not_applicable_without_a_common_scalar() {
        let reducer = JsonRepeatedValueReducer;
        let result = reducer.reduce(
            br#"[{"name":"a","status":"ok"},{"name":"b","status":"error"}]"#,
            &CompressionLimits::default(),
        );
        assert_eq!(result.metrics().status, ReductionStatus::NotApplicable);
    }
}
