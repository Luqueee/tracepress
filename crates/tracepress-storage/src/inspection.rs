use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};
use tracepress_core::RequestId;

use crate::{
    CONTEXT_INSPECTION_MAX_BLOCKS, ContextInspection, ContextInspectionBlock,
    ContextInspectionComposition, ContextInspectionCoverage, ContextInspectionNamedEstimate,
    ContextInspectionRepetition, ContextInspectionVisibility, StorageError,
};

const KIND_NAMES: [&str; 15] = [
    "instructions",
    "message",
    "text",
    "image_reference",
    "file_reference",
    "tool_definition",
    "tool_call",
    "tool_result",
    "item_reference",
    "prompt_reference",
    "provider_state_reference",
    "assistant_history",
    "opaque_reasoning",
    "opaque",
    "unknown",
];
const ROLE_NAMES: [&str; 6] = [
    "system",
    "developer",
    "user",
    "assistant",
    "tool",
    "unknown",
];
const ORIGIN_NAMES: [&str; 8] = [
    "human_authored",
    "agent_generated",
    "tool_generated",
    "tool_schema",
    "provider_managed",
    "external_reference",
    "tracepress_generated",
    "unknown",
];
const KIND_COLUMNS: [&str; 15] = [
    "estimated_tokens_kind_instructions",
    "estimated_tokens_kind_message",
    "estimated_tokens_kind_text",
    "estimated_tokens_kind_image_reference",
    "estimated_tokens_kind_file_reference",
    "estimated_tokens_kind_tool_definition",
    "estimated_tokens_kind_tool_call",
    "estimated_tokens_kind_tool_result",
    "estimated_tokens_kind_item_reference",
    "estimated_tokens_kind_prompt_reference",
    "estimated_tokens_kind_provider_state_reference",
    "estimated_tokens_kind_assistant_history",
    "estimated_tokens_kind_opaque_reasoning",
    "estimated_tokens_kind_opaque",
    "estimated_tokens_kind_unknown",
];
const ROLE_COLUMNS: [&str; 6] = [
    "estimated_tokens_role_system",
    "estimated_tokens_role_developer",
    "estimated_tokens_role_user",
    "estimated_tokens_role_assistant",
    "estimated_tokens_role_tool",
    "estimated_tokens_role_unknown",
];
const ORIGIN_COLUMNS: [&str; 8] = [
    "estimated_tokens_origin_human_authored",
    "estimated_tokens_origin_agent_generated",
    "estimated_tokens_origin_tool_generated",
    "estimated_tokens_origin_tool_schema",
    "estimated_tokens_origin_provider_managed",
    "estimated_tokens_origin_external_reference",
    "estimated_tokens_origin_tracepress_generated",
    "estimated_tokens_origin_unknown",
];

const INSPECTION_SQL: &str = "SELECT
        s.provider_request_id AS request_id,
        s.snapshot_id AS snapshot_id,
        s.session_id AS session_id,
        s.inference_operation_id AS inference_operation_id,
        COALESCE(tr.attempt_id, pa.attempt_id) AS attempt_id,
        p.provider AS provider,
        p.protocol AS protocol,
        p.request_bytes AS request_bytes,
        p.model AS model,
        s.analysis_version AS analysis_version,
        s.status AS analysis_status,
        s.correlation_status AS correlation_status,
        s.explicit_request_complete AS explicit_request_complete,
        s.uses_previous_response AS uses_previous_response,
        s.uses_conversation_state AS uses_conversation_state,
        s.uses_item_references AS uses_item_references,
        s.uses_prompt_reference AS uses_prompt_reference,
        s.uses_external_files AS uses_external_files,
        s.uses_external_images AS uses_external_images,
        s.contains_opaque_items AS contains_opaque_items,
        s.logical_context_status AS logical_context_status,
        s.duplicate_key_detected AS duplicate_key_detected,
        s.reference_resolved_locally AS reference_resolved_locally,
        s.explicit_block_count AS explicit_block_count,
        s.analyzed_bytes AS analyzed_bytes,
        s.skipped_bytes AS skipped_bytes,
        m.explicit_bytes AS explicit_bytes,
        m.estimated_tokens_kind_instructions,
        m.estimated_tokens_kind_message,
        m.estimated_tokens_kind_text,
        m.estimated_tokens_kind_image_reference,
        m.estimated_tokens_kind_file_reference,
        m.estimated_tokens_kind_tool_definition,
        m.estimated_tokens_kind_tool_call,
        m.estimated_tokens_kind_tool_result,
        m.estimated_tokens_kind_item_reference,
        m.estimated_tokens_kind_prompt_reference,
        m.estimated_tokens_kind_provider_state_reference,
        m.estimated_tokens_kind_assistant_history,
        m.estimated_tokens_kind_opaque_reasoning,
        m.estimated_tokens_kind_opaque,
        m.estimated_tokens_kind_unknown,
        m.estimated_tokens_role_system,
        m.estimated_tokens_role_developer,
        m.estimated_tokens_role_user,
        m.estimated_tokens_role_assistant,
        m.estimated_tokens_role_tool,
        m.estimated_tokens_role_unknown,
        m.estimated_tokens_origin_human_authored,
        m.estimated_tokens_origin_agent_generated,
        m.estimated_tokens_origin_tool_generated,
        m.estimated_tokens_origin_tool_schema,
        m.estimated_tokens_origin_provider_managed,
        m.estimated_tokens_origin_external_reference,
        m.estimated_tokens_origin_tracepress_generated,
        m.estimated_tokens_origin_unknown,
        m.estimated_tool_definition_share,
        m.estimated_tool_result_share,
        m.estimated_human_text_share,
        m.estimated_assistant_history_share,
        m.estimated_unique_content_share,
        m.estimated_repeated_content_share,
        m.tool_count,
        m.schema_bytes,
        m.estimated_schema_tokens,
        m.largest_tool_schema,
        m.repeated_schema_tokens,
        m.stable_explicit_prefix_estimate,
        m.estimator,
        m.estimator_version,
        m.estimate_confidence,
        m.opportunity_signals AS analysis_opportunity_signals,
        d.repeated_blocks,
        d.new_blocks,
        d.changed_blocks,
        d.removed_blocks,
        d.repeated_estimated_tokens,
        d.new_estimated_tokens,
        d.common_prefix_blocks,
        d.common_prefix_estimated_tokens,
        tr.visible_estimated_tokens,
        COALESCE(tr.provider_input_tokens, pu.input_total) AS provider_input_tokens,
        tr.residual_tokens,
        tr.comparability AS reconciliation_status
    FROM context_snapshots AS s
    JOIN provider_requests AS p ON p.request_id = s.provider_request_id
    LEFT JOIN context_analysis_metrics AS m ON m.snapshot_id = s.snapshot_id
    LEFT JOIN context_deltas AS d ON d.current_snapshot_id = s.snapshot_id
    LEFT JOIN token_reconciliations AS tr ON tr.snapshot_id = s.snapshot_id
    LEFT JOIN provider_attempts AS pa
        ON pa.request_id = s.provider_request_id
       AND pa.ordinal = (
           SELECT MAX(candidate.ordinal)
           FROM provider_attempts AS candidate
           WHERE candidate.request_id = s.provider_request_id
       )
    LEFT JOIN provider_usage AS pu
        ON pu.attempt_id = COALESCE(tr.attempt_id, pa.attempt_id)
    WHERE s.provider_request_id = ?1
    ORDER BY s.analysis_version DESC, s.snapshot_id DESC
    LIMIT 1";

struct RawSnapshot {
    request_id: String,
    snapshot_id: String,
    session_id: String,
    inference_operation_id: String,
    attempt_id: Option<String>,
    provider: Option<String>,
    protocol: Option<String>,
    request_bytes: Option<i64>,
    model: Option<String>,
    analysis_version: i64,
    analysis_status: String,
    correlation_status: Option<String>,
    visibility: RawVisibility,
    coverage: RawCoverage,
    kind_tokens: Vec<Option<i64>>,
    role_tokens: Vec<Option<i64>>,
    origin_tokens: Vec<Option<i64>>,
    composition: RawComposition,
    repetition: RawRepetition,
    visible_estimated_tokens: Option<i64>,
    provider_input_tokens: Option<i64>,
    residual_tokens: Option<i64>,
    reconciliation_status: Option<String>,
}

struct RawVisibility {
    explicit_request_complete: Option<i64>,
    uses_previous_response: Option<i64>,
    uses_conversation_state: Option<i64>,
    uses_item_references: Option<i64>,
    uses_prompt_reference: Option<i64>,
    uses_external_files: Option<i64>,
    uses_external_images: Option<i64>,
    contains_opaque_items: Option<i64>,
    logical_context_status: Option<String>,
    duplicate_key_detected: Option<i64>,
    reference_resolved_locally: Option<i64>,
}

struct RawCoverage {
    explicit_block_count: Option<i64>,
    analyzed_bytes: Option<i64>,
    skipped_bytes: Option<i64>,
    explicit_bytes: Option<i64>,
}

struct RawComposition {
    estimated_tool_definition_share: Option<f64>,
    estimated_tool_result_share: Option<f64>,
    estimated_human_text_share: Option<f64>,
    estimated_assistant_history_share: Option<f64>,
    estimated_unique_content_share: Option<f64>,
    estimated_repeated_content_share: Option<f64>,
    tool_count: Option<i64>,
    schema_bytes: Option<i64>,
    estimated_schema_tokens: Option<i64>,
    largest_tool_schema: Option<i64>,
    repeated_schema_tokens: Option<i64>,
    opportunity_signals: Option<String>,
    estimator: Option<String>,
    estimator_version: Option<i64>,
    estimate_confidence: Option<String>,
    stable_explicit_prefix_estimate: Option<i64>,
}

struct RawRepetition {
    repeated_blocks: Option<i64>,
    new_blocks: Option<i64>,
    changed_blocks: Option<i64>,
    removed_blocks: Option<i64>,
    repeated_estimated_tokens: Option<i64>,
    new_estimated_tokens: Option<i64>,
    common_prefix_blocks: Option<i64>,
    common_prefix_estimated_tokens: Option<i64>,
}

/// Runs the metadata-only context inspection on the daemon-owned writer connection.
#[allow(
    clippy::redundant_pub_crate,
    reason = "the module remains crate-private while the writer sibling calls this boundary"
)]
pub(crate) fn query_context_inspection(
    connection: &Connection,
    request_id: RequestId,
) -> Result<ContextInspection, StorageError> {
    let request_text = request_id.to_string();
    let raw = crate::encode::sqlite(
        connection
            .query_row(INSPECTION_SQL, params![request_text], read_snapshot)
            .optional(),
    )?
    .ok_or(StorageError::ContextSnapshotNotFound { request_id })?;

    let blocks = read_largest_blocks(connection, &raw.snapshot_id)?;
    build_inspection(raw, blocks)
}

fn read_snapshot(row: &Row<'_>) -> rusqlite::Result<RawSnapshot> {
    Ok(RawSnapshot {
        request_id: row.get("request_id")?,
        snapshot_id: row.get("snapshot_id")?,
        session_id: row.get("session_id")?,
        inference_operation_id: row.get("inference_operation_id")?,
        attempt_id: row.get("attempt_id")?,
        provider: row.get("provider")?,
        protocol: row.get("protocol")?,
        request_bytes: row.get("request_bytes")?,
        model: row.get("model")?,
        analysis_version: row.get("analysis_version")?,
        analysis_status: row.get("analysis_status")?,
        correlation_status: row.get("correlation_status")?,
        visibility: RawVisibility {
            explicit_request_complete: row.get("explicit_request_complete")?,
            uses_previous_response: row.get("uses_previous_response")?,
            uses_conversation_state: row.get("uses_conversation_state")?,
            uses_item_references: row.get("uses_item_references")?,
            uses_prompt_reference: row.get("uses_prompt_reference")?,
            uses_external_files: row.get("uses_external_files")?,
            uses_external_images: row.get("uses_external_images")?,
            contains_opaque_items: row.get("contains_opaque_items")?,
            logical_context_status: row.get("logical_context_status")?,
            duplicate_key_detected: row.get("duplicate_key_detected")?,
            reference_resolved_locally: row.get("reference_resolved_locally")?,
        },
        coverage: RawCoverage {
            explicit_block_count: row.get("explicit_block_count")?,
            analyzed_bytes: row.get("analyzed_bytes")?,
            skipped_bytes: row.get("skipped_bytes")?,
            explicit_bytes: row.get("explicit_bytes")?,
        },
        kind_tokens: KIND_COLUMNS
            .iter()
            .map(|column| row.get(*column))
            .collect::<rusqlite::Result<Vec<Option<i64>>>>()?,
        role_tokens: ROLE_COLUMNS
            .iter()
            .map(|column| row.get(*column))
            .collect::<rusqlite::Result<Vec<Option<i64>>>>()?,
        origin_tokens: ORIGIN_COLUMNS
            .iter()
            .map(|column| row.get(*column))
            .collect::<rusqlite::Result<Vec<Option<i64>>>>()?,
        composition: RawComposition {
            estimated_tool_definition_share: row.get("estimated_tool_definition_share")?,
            estimated_tool_result_share: row.get("estimated_tool_result_share")?,
            estimated_human_text_share: row.get("estimated_human_text_share")?,
            estimated_assistant_history_share: row.get("estimated_assistant_history_share")?,
            estimated_unique_content_share: row.get("estimated_unique_content_share")?,
            estimated_repeated_content_share: row.get("estimated_repeated_content_share")?,
            tool_count: row.get("tool_count")?,
            schema_bytes: row.get("schema_bytes")?,
            estimated_schema_tokens: row.get("estimated_schema_tokens")?,
            largest_tool_schema: row.get("largest_tool_schema")?,
            repeated_schema_tokens: row.get("repeated_schema_tokens")?,
            opportunity_signals: row.get("analysis_opportunity_signals")?,
            estimator: row.get("estimator")?,
            estimator_version: row.get("estimator_version")?,
            estimate_confidence: row.get("estimate_confidence")?,
            stable_explicit_prefix_estimate: row.get("stable_explicit_prefix_estimate")?,
        },
        repetition: RawRepetition {
            repeated_blocks: row.get("repeated_blocks")?,
            new_blocks: row.get("new_blocks")?,
            changed_blocks: row.get("changed_blocks")?,
            removed_blocks: row.get("removed_blocks")?,
            repeated_estimated_tokens: row.get("repeated_estimated_tokens")?,
            new_estimated_tokens: row.get("new_estimated_tokens")?,
            common_prefix_blocks: row.get("common_prefix_blocks")?,
            common_prefix_estimated_tokens: row.get("common_prefix_estimated_tokens")?,
        },
        visible_estimated_tokens: row.get("visible_estimated_tokens")?,
        provider_input_tokens: row.get("provider_input_tokens")?,
        residual_tokens: row.get("residual_tokens")?,
        reconciliation_status: row.get("reconciliation_status")?,
    })
}

fn read_largest_blocks(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Vec<ContextInspectionBlock>, StorageError> {
    let mut statement = crate::encode::sqlite(connection.prepare(
        "SELECT ordinal, kind, role, origin, raw_bytes, estimated_tokens, detected_kind,
                detector_confidence, detector_version, opportunity_signals,
                candidate_estimated_tokens, repetition_score
         FROM context_block_occurrences
         WHERE snapshot_id = ?1
         ORDER BY raw_bytes DESC, ordinal ASC, block_occurrence_id ASC
         LIMIT ?2",
    ))?;
    let limit = i64::try_from(CONTEXT_INSPECTION_MAX_BLOCKS).map_err(|_| {
        StorageError::InvalidContextInspection {
            field: "largest_blocks_limit",
        }
    })?;
    let rows = crate::encode::sqlite(statement.query_map(params![snapshot_id, limit], read_block))?;
    crate::encode::sqlite(rows.collect::<rusqlite::Result<Vec<_>>>())
}

fn read_block(row: &Row<'_>) -> rusqlite::Result<ContextInspectionBlock> {
    Ok(ContextInspectionBlock {
        ordinal: row_u64(row.get("ordinal")?)?,
        kind: bounded_required(&row.get::<_, String>("kind")?, 64),
        role: bounded_required(&row.get::<_, String>("role")?, 32),
        origin: bounded_required(&row.get::<_, String>("origin")?, 64),
        raw_bytes: row_u64(row.get("raw_bytes")?)?,
        estimated_tokens: row_optional_u64(row.get("estimated_tokens")?)?,
        detected_kind: row
            .get::<_, Option<String>>("detected_kind")?
            .map(|value| bounded_text(&value, 64)),
        detector_confidence: row.get("detector_confidence")?,
        detector_version: row_optional_u32(row.get("detector_version")?)?,
        opportunity_signals: decode_signals(row.get("opportunity_signals")?),
        candidate_estimated_tokens: row_optional_u64(row.get("candidate_estimated_tokens")?)?,
        repetition_score: row.get("repetition_score")?,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "the bounded projection keeps every metadata field explicit"
)]
fn build_inspection(
    raw: RawSnapshot,
    largest_blocks: Vec<ContextInspectionBlock>,
) -> Result<ContextInspection, StorageError> {
    let kind_tokens = optional_u64_vec(raw.kind_tokens, "estimated_tokens_by_kind")?;
    let role_tokens = optional_u64_vec(raw.role_tokens, "estimated_tokens_by_role")?;
    let origin_tokens = optional_u64_vec(raw.origin_tokens, "estimated_tokens_by_origin")?;
    let analysis_version = u32::try_from(raw.analysis_version).map_err(|_| {
        StorageError::InvalidContextInspection {
            field: "analysis_version",
        }
    })?;

    let composition = raw.composition;
    let stable_explicit_prefix_estimate = optional_u64(
        composition.stable_explicit_prefix_estimate,
        "stable_explicit_prefix_estimate",
    )?;
    let inspection = ContextInspection {
        request_id: parse_id(&raw.request_id, "request_id")?,
        snapshot_id: parse_id(&raw.snapshot_id, "snapshot_id")?,
        session_id: parse_id(&raw.session_id, "session_id")?,
        inference_operation_id: parse_id(&raw.inference_operation_id, "inference_operation_id")?,
        attempt_id: raw
            .attempt_id
            .as_deref()
            .map(|value| parse_id(value, "attempt_id"))
            .transpose()?,
        provider: bounded_optional(raw.provider.as_deref(), 64),
        protocol: bounded_optional(raw.protocol.as_deref(), 96),
        request_bytes: optional_u64(raw.request_bytes, "request_bytes")?,
        model: bounded_optional(raw.model.as_deref(), 128),
        analysis_version,
        analysis_status: bounded_required(&raw.analysis_status, 64),
        correlation_status: bounded_optional(raw.correlation_status.as_deref(), 64),
        visibility: ContextInspectionVisibility {
            explicit_request_complete: optional_bool(
                raw.visibility.explicit_request_complete,
                "explicit_request_complete",
            )?,
            uses_previous_response: optional_bool(
                raw.visibility.uses_previous_response,
                "uses_previous_response",
            )?,
            uses_conversation_state: optional_bool(
                raw.visibility.uses_conversation_state,
                "uses_conversation_state",
            )?,
            uses_item_references: optional_bool(
                raw.visibility.uses_item_references,
                "uses_item_references",
            )?,
            uses_prompt_reference: optional_bool(
                raw.visibility.uses_prompt_reference,
                "uses_prompt_reference",
            )?,
            uses_external_files: optional_bool(
                raw.visibility.uses_external_files,
                "uses_external_files",
            )?,
            uses_external_images: optional_bool(
                raw.visibility.uses_external_images,
                "uses_external_images",
            )?,
            contains_opaque_items: optional_bool(
                raw.visibility.contains_opaque_items,
                "contains_opaque_items",
            )?,
            logical_context_status: logical_context_status(&raw.visibility),
            duplicate_key_detected: optional_bool(
                raw.visibility.duplicate_key_detected,
                "duplicate_key_detected",
            )?,
            reference_resolved_locally: optional_bool(
                raw.visibility.reference_resolved_locally,
                "reference_resolved_locally",
            )?,
        },
        provider_input_tokens: optional_u64(raw.provider_input_tokens, "provider_input_tokens")?,
        visible_estimated_tokens: optional_u64(
            raw.visible_estimated_tokens,
            "visible_estimated_tokens",
        )?,
        estimator: bounded_optional(composition.estimator.as_deref(), 64),
        estimator_version: optional_u32(composition.estimator_version, "estimator_version")?,
        estimate_confidence: bounded_optional(composition.estimate_confidence.as_deref(), 64),
        reconciliation_status: bounded_optional(raw.reconciliation_status.as_deref(), 64),
        residual_tokens: raw.residual_tokens,
        composition: ContextInspectionComposition {
            by_kind: named_estimates(&KIND_NAMES, kind_tokens),
            by_role: named_estimates(&ROLE_NAMES, role_tokens),
            by_origin: named_estimates(&ORIGIN_NAMES, origin_tokens),
            estimated_tool_definition_share: composition.estimated_tool_definition_share,
            estimated_tool_result_share: composition.estimated_tool_result_share,
            estimated_human_text_share: composition.estimated_human_text_share,
            estimated_assistant_history_share: composition.estimated_assistant_history_share,
            estimated_unique_content_share: composition.estimated_unique_content_share,
            estimated_repeated_content_share: composition.estimated_repeated_content_share,
            tool_count: optional_u64(composition.tool_count, "tool_count")?,
            schema_bytes: optional_u64(composition.schema_bytes, "schema_bytes")?,
            estimated_schema_tokens: optional_u64(
                composition.estimated_schema_tokens,
                "estimated_schema_tokens",
            )?,
            largest_tool_schema: optional_u64(
                composition.largest_tool_schema,
                "largest_tool_schema",
            )?,
            repeated_schema_tokens: optional_u64(
                composition.repeated_schema_tokens,
                "repeated_schema_tokens",
            )?,
            opportunity_signals: decode_signals(composition.opportunity_signals),
        },
        repetition: ContextInspectionRepetition {
            repeated_blocks: optional_u64(raw.repetition.repeated_blocks, "repeated_blocks")?,
            new_blocks: optional_u64(raw.repetition.new_blocks, "new_blocks")?,
            changed_blocks: optional_u64(raw.repetition.changed_blocks, "changed_blocks")?,
            removed_blocks: optional_u64(raw.repetition.removed_blocks, "removed_blocks")?,
            repeated_estimated_tokens: optional_u64(
                raw.repetition.repeated_estimated_tokens,
                "repeated_estimated_tokens",
            )?,
            new_estimated_tokens: optional_u64(
                raw.repetition.new_estimated_tokens,
                "new_estimated_tokens",
            )?,
            common_prefix_blocks: optional_u64(
                raw.repetition.common_prefix_blocks,
                "common_prefix_blocks",
            )?,
            common_prefix_estimated_tokens: optional_u64(
                raw.repetition.common_prefix_estimated_tokens,
                "common_prefix_estimated_tokens",
            )?,
        },
        stable_explicit_prefix_estimate,
        coverage: ContextInspectionCoverage {
            explicit_block_count: optional_u64(
                raw.coverage.explicit_block_count,
                "explicit_block_count",
            )?,
            analyzed_bytes: optional_u64(raw.coverage.analyzed_bytes, "analyzed_bytes")?,
            skipped_bytes: optional_u64(raw.coverage.skipped_bytes, "skipped_bytes")?,
            explicit_bytes: optional_u64(raw.coverage.explicit_bytes, "explicit_bytes")?,
        },
        largest_blocks,
    };
    Ok(inspection)
}

fn named_estimates(
    names: &[&str],
    values: Vec<Option<u64>>,
) -> Vec<ContextInspectionNamedEstimate> {
    names
        .iter()
        .zip(values)
        .map(|(name, estimated_tokens)| ContextInspectionNamedEstimate {
            name: (*name).to_owned(),
            estimated_tokens,
        })
        .collect()
}

fn optional_u64(value: Option<i64>, field: &'static str) -> Result<Option<u64>, StorageError> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| StorageError::InvalidContextInspection { field })
        })
        .transpose()
}

fn optional_u64_vec(
    values: Vec<Option<i64>>,
    field: &'static str,
) -> Result<Vec<Option<u64>>, StorageError> {
    values
        .into_iter()
        .map(|value| optional_u64(value, field))
        .collect()
}

fn optional_u32(value: Option<i64>, field: &'static str) -> Result<Option<u32>, StorageError> {
    value
        .map(|value| {
            u32::try_from(value).map_err(|_| StorageError::InvalidContextInspection { field })
        })
        .transpose()
}

fn optional_bool(value: Option<i64>, field: &'static str) -> Result<Option<bool>, StorageError> {
    value
        .map(|value| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StorageError::InvalidContextInspection { field }),
        })
        .transpose()
}

fn logical_context_status(visibility: &RawVisibility) -> Option<String> {
    let provider_managed = [
        visibility.uses_previous_response,
        visibility.uses_conversation_state,
        visibility.uses_item_references,
        visibility.uses_prompt_reference,
    ]
    .into_iter()
    .any(|value| value == Some(1));
    let external = [
        visibility.uses_external_files,
        visibility.uses_external_images,
    ]
    .into_iter()
    .any(|value| value == Some(1));
    match (provider_managed, external) {
        (true, true) => Some("mixed_partial".to_owned()),
        (true, false) => Some("provider_managed_partial".to_owned()),
        (false, true) => Some("external_references_partial".to_owned()),
        (false, false) => bounded_optional(visibility.logical_context_status.as_deref(), 64)
            .filter(|value| value != "complete"),
    }
}

fn parse_id<T>(value: &str, field: &'static str) -> Result<T, StorageError>
where
    T: FromStr,
{
    value
        .parse()
        .map_err(|_error| StorageError::InvalidContextInspection { field })
}

fn row_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error))
    })
}

fn row_optional_u64(value: Option<i64>) -> rusqlite::Result<Option<u64>> {
    value.map(row_u64).transpose()
}

fn row_optional_u32(value: Option<i64>) -> rusqlite::Result<Option<u32>> {
    value
        .map(|value| {
            u32::try_from(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error))
            })
        })
        .transpose()
}

fn bounded_required(value: &str, maximum: usize) -> String {
    let value = bounded_text(value, maximum);
    if value.is_empty() {
        "unknown".to_owned()
    } else {
        value
    }
}

fn bounded_optional(value: Option<&str>, maximum: usize) -> Option<String> {
    value
        .map(|value| bounded_text(value, maximum))
        .filter(|value| !value.is_empty())
}

fn bounded_text(value: &str, maximum: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(maximum)
        .collect()
}

fn decode_signals(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .filter_map(|signal| match signal {
            "large_tool_result"
            | "high_duplication"
            | "homogeneous_json"
            | "repetitive_logs"
            | "large_search_result"
            | "large_test_output"
            | "large_tool_schema"
            | "repeated_history" => Some(signal.to_owned()),
            _ => None,
        })
        .take(8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::decode_signals;

    #[test]
    fn decode_signals_only_keeps_bounded_structural_names() {
        assert_eq!(
            decode_signals(Some(
                "large_tool_result,not-content,high_duplication".to_owned(),
            )),
            vec!["large_tool_result", "high_duplication"]
        );
    }
}
