//! Daemon diagnostics and human-readable context inspection output.

use super::*;

pub(super) async fn doctor(config: &Config) -> Result<(), String> {
    config.ensure_root()?;
    let _credential = credential(config)?;
    println!("state: {}", config.root.display());
    println!(
        "daemon: {}",
        if daemon_running(config).await? {
            "running"
        } else {
            "stopped"
        }
    );
    Ok(())
}

pub(super) async fn context(config: &Config, request_id: RequestId) -> Result<(), String> {
    match control(config, ControlRequest::Context { request_id }).await? {
        ControlResponse::Context { inspection } => print_context_inspection(&inspection),
        ControlResponse::Error { message } => return Err(message),
        ControlResponse::Ok { .. } | ControlResponse::ContextStatus { .. } => {
            return Err("daemon returned an unexpected context response".to_owned());
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "the command intentionally renders the stable inspection sections together"
)]
pub(super) fn print_context_inspection(inspection: &ContextInspection) {
    println!("Context visibility");
    println!("  request id: {}", inspection.request_id);
    println!("  snapshot id: {}", inspection.snapshot_id);
    println!(
        "  explicit request complete: {}",
        bool_text(inspection.visibility.explicit_request_complete)
    );
    println!(
        "  logical context status: {}",
        optional_text(inspection.visibility.logical_context_status.as_ref())
    );
    println!(
        "  previous response state: {}",
        bool_text(inspection.visibility.uses_previous_response)
    );
    println!(
        "  conversation state: {}",
        bool_text(inspection.visibility.uses_conversation_state)
    );
    println!(
        "  item references: {}",
        bool_text(inspection.visibility.uses_item_references)
    );
    println!(
        "  prompt reference: {}",
        bool_text(inspection.visibility.uses_prompt_reference)
    );
    println!(
        "  external files: {}",
        bool_text(inspection.visibility.uses_external_files)
    );
    println!(
        "  external images: {}",
        bool_text(inspection.visibility.uses_external_images)
    );
    println!(
        "  opaque items: {}",
        bool_text(inspection.visibility.contains_opaque_items)
    );
    println!(
        "  duplicate keys: {}",
        bool_text(inspection.visibility.duplicate_key_detected)
    );
    println!(
        "  references resolved locally: {}",
        bool_text(inspection.visibility.reference_resolved_locally)
    );

    println!("\nProvider input observed");
    println!(
        "  provider: {}",
        optional_text(inspection.provider.as_ref())
    );
    println!(
        "  protocol: {}",
        optional_text(inspection.protocol.as_ref())
    );
    println!("  model: {}", optional_text(inspection.model.as_ref()));
    println!(
        "  accepted request bytes: {}",
        optional_number(inspection.request_bytes)
    );
    println!(
        "  input tokens observed: {}",
        optional_number(inspection.provider_input_tokens)
    );

    println!("\nVisible estimate + estimator + reconciliation/residual");
    println!(
        "  visible tokens (estimated): {}",
        estimated_number(inspection.visible_estimated_tokens)
    );
    println!(
        "  estimator: {}",
        optional_text(inspection.estimator.as_ref())
    );
    println!(
        "  estimator version: {}",
        optional_number(inspection.estimator_version.map(u64::from))
    );
    println!(
        "  estimate confidence: {}",
        optional_text(inspection.estimate_confidence.as_ref())
    );
    println!(
        "  reconciliation status: {}",
        optional_text(inspection.reconciliation_status.as_ref())
    );
    println!(
        "  residual (provider minus visible estimate, signed): {}",
        signed_number(inspection.residual_tokens)
    );

    println!("\nComposition (estimated)");
    print_named_estimates("  by kind", &inspection.composition.by_kind);
    print_named_estimates("  by role", &inspection.composition.by_role);
    print_named_estimates("  by origin", &inspection.composition.by_origin);
    print_share(
        "  tool definition share (estimated)",
        inspection.composition.estimated_tool_definition_share,
    );
    print_share(
        "  tool result share (estimated)",
        inspection.composition.estimated_tool_result_share,
    );
    print_share(
        "  human text share (estimated)",
        inspection.composition.estimated_human_text_share,
    );
    print_share(
        "  assistant history share (estimated)",
        inspection.composition.estimated_assistant_history_share,
    );
    print_share(
        "  unique content share (estimated)",
        inspection.composition.estimated_unique_content_share,
    );
    print_share(
        "  repeated content share (estimated)",
        inspection.composition.estimated_repeated_content_share,
    );
    println!(
        "  tool count: {}",
        optional_number(inspection.composition.tool_count)
    );
    println!(
        "  schema bytes: {}",
        optional_number(inspection.composition.schema_bytes)
    );
    println!(
        "  schema tokens (estimated): {}",
        estimated_number(inspection.composition.estimated_schema_tokens)
    );
    println!(
        "  largest tool schema bytes: {}",
        optional_number(inspection.composition.largest_tool_schema)
    );
    println!(
        "  repeated schema tokens (estimated): {}",
        estimated_number(inspection.composition.repeated_schema_tokens)
    );
    println!(
        "  opportunity signals: {}",
        if inspection.composition.opportunity_signals.is_empty() {
            "none".to_owned()
        } else {
            inspection.composition.opportunity_signals.join(", ")
        }
    );

    println!("\nRepetition");
    println!(
        "  repeated blocks: {}",
        optional_number(inspection.repetition.repeated_blocks)
    );
    println!(
        "  new blocks: {}",
        optional_number(inspection.repetition.new_blocks)
    );
    println!(
        "  changed blocks: {}",
        optional_number(inspection.repetition.changed_blocks)
    );
    println!(
        "  removed blocks: {}",
        optional_number(inspection.repetition.removed_blocks)
    );
    println!(
        "  repeated tokens (estimated): {}",
        estimated_number(inspection.repetition.repeated_estimated_tokens)
    );
    println!(
        "  new tokens (estimated): {}",
        estimated_number(inspection.repetition.new_estimated_tokens)
    );

    println!("\nStable explicit prefix");
    println!(
        "  common-prefix blocks: {}",
        optional_number(inspection.repetition.common_prefix_blocks)
    );
    println!(
        "  common-prefix tokens (estimated): {}",
        estimated_number(inspection.repetition.common_prefix_estimated_tokens)
    );
    println!(
        "  stable explicit-prefix estimate (estimated): {}",
        estimated_number(inspection.stable_explicit_prefix_estimate)
    );

    println!("\nLargest blocks");
    if inspection.largest_blocks.is_empty() {
        println!("  unavailable");
    } else {
        for block in &inspection.largest_blocks {
            print_block(block);
        }
    }

    println!("\nAnalysis");
    println!("  status: {}", analysis_status_text(inspection));
    println!("  version: {}", inspection.analysis_version);
    println!(
        "  explicit blocks: {}",
        optional_number(inspection.coverage.explicit_block_count)
    );
    println!(
        "  analyzed bytes: {}",
        optional_number(inspection.coverage.analyzed_bytes)
    );
    println!(
        "  skipped bytes: {}",
        optional_number(inspection.coverage.skipped_bytes)
    );
    println!(
        "  explicit bytes: {}",
        optional_number(inspection.coverage.explicit_bytes)
    );
    println!(
        "  unknown blocks: {}",
        optional_number(inspection.coverage.unknown_block_count)
    );
    println!(
        "  semantic coverage: {}",
        inspection
            .coverage
            .semantic_coverage_basis_points
            .map_or_else(
                || "unknown".to_owned(),
                |value| { format!("{:.2}%", f64::from(value) / 100.0) }
            )
    );

    println!("\nCorrelation");
    println!(
        "  context correlation: {}",
        optional_text(inspection.correlation_status.as_ref())
    );
    println!(
        "  attempt id: {}",
        inspection
            .attempt_id
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
    );
}

pub(super) fn analysis_status_text(inspection: &ContextInspection) -> String {
    if inspection.analysis_status == "complete"
        && inspection
            .visibility
            .logical_context_status
            .as_deref()
            .is_some_and(|status| status.ends_with("_partial"))
    {
        "finalized (logical context partial)".to_owned()
    } else {
        inspection.analysis_status.clone()
    }
}

pub(super) fn print_named_estimates(label: &str, values: &[ContextInspectionNamedEstimate]) {
    println!("{label}:");
    for value in values {
        println!(
            "    {}: {} estimated tokens",
            value.name,
            estimated_number(value.estimated_tokens)
        );
    }
}

pub(super) fn print_share(label: &str, value: Option<f64>) {
    println!(
        "{label}: {}",
        value.map_or_else(|| "unknown".to_owned(), |value| format!("{value:.3}"))
    );
}

pub(super) fn print_block(block: &ContextInspectionBlock) {
    println!(
        "  ordinal {}: kind={} role={} origin={} raw bytes={}",
        block.ordinal, block.kind, block.role, block.origin, block.raw_bytes
    );
    println!(
        "    estimated tokens: {}",
        estimated_number(block.estimated_tokens)
    );
    println!(
        "    detection: {}",
        optional_text(block.detected_kind.as_ref())
    );
    println!(
        "    detector confidence: {}",
        block
            .detector_confidence
            .map_or_else(|| "unknown".to_owned(), |value| format!("{value:.3}"))
    );
    println!(
        "    detector version: {}",
        optional_number(block.detector_version.map(u64::from))
    );
    println!(
        "    repetition score: {}",
        block
            .repetition_score
            .map_or_else(|| "unknown".to_owned(), |value| format!("{value:.3}"))
    );
    println!(
        "    opportunity signals: {}",
        if block.opportunity_signals.is_empty() {
            "none".to_owned()
        } else {
            block.opportunity_signals.join(", ")
        }
    );
    println!(
        "    candidate tokens (estimated): {}",
        estimated_number(block.candidate_estimated_tokens)
    );
}

pub(super) fn optional_text(value: Option<&String>) -> String {
    value.cloned().unwrap_or_else(|| "unknown".to_owned())
}

pub(super) fn optional_number(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

pub(super) fn estimated_number(value: Option<u64>) -> String {
    optional_number(value)
}

pub(super) fn signed_number(value: Option<i64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| format!("{value:+}"))
}

pub(super) const fn bool_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}
