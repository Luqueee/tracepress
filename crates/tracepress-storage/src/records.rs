#![allow(
    clippy::redundant_pub_crate,
    reason = "the writer sibling module consumes these transaction functions"
)]

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::{
    ContextAnalysisStatus, ContextBlockKind, ContextOrigin, ContextRole, RecoveryReceipt,
    StorageError, WriteBatch, WriteCommand, WriteReceipt,
    encode::{
        causal_relationship, content_kind, content_role, context_analysis_status_text,
        context_block_kind_text, context_correlation_status_text, context_origin_text,
        context_role_text, detected_content_kind_text, estimate_confidence_text, fidelity_class,
        inference_status, logical_context_status_text, observation_status_text, operation_kind,
        operation_status, opportunity_signals_text, provider_kind_text, provider_protocol_text,
        reconciliation_status_text, request_method, request_route, response_state_text,
        session_state, sqlite, sqlite_optional, sqlite_u64, usage_status_text,
    },
};

use tracepress_core::{EventId, UuidV7Generator};

pub(crate) fn execute_single(
    connection: &mut Connection,
    command: &WriteCommand,
) -> Result<WriteReceipt, StorageError> {
    let transaction = sqlite(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let receipt = insert(&transaction, command)?;
    sqlite(transaction.commit())?;
    Ok(receipt)
}
pub(crate) fn recover_stale_sessions(
    connection: &mut Connection,
    recovered_at: &str,
) -> Result<RecoveryReceipt, StorageError> {
    let transaction = sqlite(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let recovered_sessions = u64::try_from(sqlite(transaction.execute(
        "UPDATE sessions SET state = ?1, ended_at = COALESCE(ended_at, ?2) WHERE state IN (?3, ?4)",
        params![
            session_state(tracepress_core::SessionState::Stale),
            recovered_at,
            session_state(tracepress_core::SessionState::Active),
            session_state(tracepress_core::SessionState::Closing)
        ],
    ))?)
    .map_err(|_error| StorageError::IntegerOverflow {
        field: "recovered_sessions",
        value: u64::MAX,
    })?;

    // A Begin has no terminal outcome yet, so `completed_at_us IS NULL` is the durable marker of
    // an interrupted analysis. A recovery marker makes this transition idempotent without
    // inspecting event payloads: finalized partial analyses have a completion timestamp, while a
    // recovered snapshot keeps `completed_at_us` NULL and is excluded from later recoveries.
    let snapshots = {
        let mut statement = sqlite(transaction.prepare(
            "SELECT snapshot_id, session_id, inference_operation_id \
             FROM context_snapshots \
             WHERE completed_at_us IS NULL AND recovered_at_us IS NULL",
        ))?;
        let result = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        let result = sqlite(result)?;
        let mut snapshots = Vec::new();
        for snapshot in result {
            snapshots.push(sqlite(snapshot)?);
        }
        snapshots
    };
    let recovered_at_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let recovered_at_us = recovered_at_us
        .as_secs()
        .saturating_mul(1_000_000)
        .saturating_add(u64::from(recovered_at_us.subsec_micros()));
    let recovered_at_us = sqlite_u64(recovered_at_us, "recovered_at_us")?;
    let generator = UuidV7Generator::new();
    let mut recovered_context_snapshots = 0_u64;
    for (snapshot_id, session_id, operation_id) in snapshots {
        let changed = sqlite(transaction.execute(
            "UPDATE context_snapshots SET status = ?2, recovered_at_us = ?3 \
             WHERE snapshot_id = ?1 AND completed_at_us IS NULL AND recovered_at_us IS NULL",
            params![
                snapshot_id,
                context_analysis_status_text(ContextAnalysisStatus::Partial),
                recovered_at_us,
            ],
        ))?;
        if changed == 0 {
            continue;
        }
        recovered_context_snapshots = recovered_context_snapshots.saturating_add(1);
        let event_id = EventId::generate(&generator);
        let payload = format!(r#"{{"reason":"startup_recovery","snapshot_id":"{snapshot_id}"}}"#)
            .into_bytes()
            .into_boxed_slice();
        let _event_rows = sqlite(transaction.execute(
            "INSERT INTO events(event_id, session_id, operation_id, timestamp, event_type, payload, schema_version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event_id.to_string(),
                session_id,
                operation_id,
                recovered_at,
                "context.analysis.partial",
                payload.as_ref(),
                "1",
            ],
        ))?;
    }
    sqlite(transaction.commit())?;
    Ok(RecoveryReceipt {
        recovered_sessions,
        recovered_context_snapshots,
    })
}

pub(crate) fn execute_batch(
    connection: &mut Connection,
    batch: &WriteBatch,
) -> Result<WriteReceipt, StorageError> {
    execute_batch_inner(
        connection,
        batch,
        #[cfg(test)]
        None,
    )
}

#[cfg(test)]
pub(crate) fn execute_interrupted_batch(
    connection: &mut Connection,
    batch: &WriteBatch,
    fail_after: usize,
) -> Result<WriteReceipt, StorageError> {
    execute_batch_inner(connection, batch, Some(fail_after))
}

fn execute_batch_inner(
    connection: &mut Connection,
    batch: &WriteBatch,
    #[cfg(test)] fail_after: Option<usize>,
) -> Result<WriteReceipt, StorageError> {
    let transaction = sqlite(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    #[cfg(test)]
    for (index, command) in batch.commands.iter().enumerate() {
        let _receipt = insert(&transaction, command)?;
        if fail_after == index.checked_add(1) {
            return Err(StorageError::InjectedFailure);
        }
    }
    #[cfg(not(test))]
    for command in &batch.commands {
        let _receipt = insert(&transaction, command)?;
    }
    sqlite(transaction.commit())?;
    let rows_changed =
        u64::try_from(batch.commands.len()).map_err(|_error| StorageError::IntegerOverflow {
            field: "batch_rows_changed",
            value: u64::MAX,
        })?;
    Ok(WriteReceipt::BatchCommitted { rows_changed })
}

#[allow(
    clippy::too_many_lines,
    reason = "the exhaustive command-to-table dispatcher mirrors the canonical schema without branching"
)]
fn insert(
    transaction: &Transaction<'_>,
    command: &WriteCommand,
) -> Result<WriteReceipt, StorageError> {
    match command {
        WriteCommand::Session {
            session_id,
            started_at,
            ended_at,
            state,
            ingress_key,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO sessions(session_id, started_at, ended_at, state, ingress_key) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id.to_string(), started_at, ended_at, session_state(*state), ingress_key],
        ))?),
        WriteCommand::SessionState {
            session_id,
            ended_at,
            state,
        } => committed(sqlite(transaction.execute(
            "UPDATE sessions SET ended_at = ?2, state = ?3 WHERE session_id = ?1",
            params![session_id.to_string(), ended_at, session_state(*state)],
        ))?),
        WriteCommand::Operation {
            operation_id,
            session_id,
            kind,
            started_at,
            ended_at,
            status,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO operations(operation_id, session_id, kind, started_at, ended_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![operation_id.to_string(), session_id.to_string(), operation_kind(*kind), started_at, ended_at, operation_status(*status)],
        ))?),
        WriteCommand::OperationState {
            operation_id,
            ended_at,
            status,
        } => committed(sqlite(transaction.execute(
            "UPDATE operations SET ended_at = ?2, status = ?3 WHERE operation_id = ?1",
            params![operation_id.to_string(), ended_at, operation_status(*status)],
        ))?),
        WriteCommand::CausalEdge { edge } => committed(sqlite(transaction.execute(
            "INSERT INTO causal_edges(parent_operation_id, child_operation_id, relationship) VALUES (?1, ?2, ?3)",
            params![edge.parent_operation_id().to_string(), edge.child_operation_id().to_string(), causal_relationship(edge.relationship())],
        ))?),
        WriteCommand::ProviderRequest {
            operation_id,
            metadata,
            provider,
            protocol,
            parser_version,
            observation_status,
            model,
            stream,
            background,
            store,
            reasoning_effort,
            text_verbosity,
            truncation,
            previous_response_id_present,
            input_item_count,
            tool_count,
            text_input_block_count,
            image_input_block_count,
            file_input_block_count,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO provider_requests(request_id, operation_id, route, method, request_bytes, provider, protocol, parser_version, observation_status, model, stream, background, store, reasoning_effort, text_verbosity, truncation, previous_response_id_present, input_item_count, tool_count, text_input_block_count, image_input_block_count, file_input_block_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                metadata.request_id().to_string(),
                operation_id.to_string(),
                request_route(metadata.route()),
                request_method(metadata.method()),
                sqlite_u64(metadata.request_bytes(), "request_bytes")?,
                provider.as_ref().map(|value| provider_kind_text(*value)),
                protocol.as_ref().map(|value| provider_protocol_text(*value)),
                (*parser_version).map(|value| sqlite_u64(u64::from(value), "parser_version")).transpose()?,
                observation_status.as_ref().map(|value| observation_status_text(*value)),
                model.as_deref(),
                stream,
                background,
                store,
                reasoning_effort.as_deref(),
                text_verbosity.as_deref(),
                truncation.as_deref(),
                previous_response_id_present,
                (*input_item_count).map(|value| sqlite_u64(value, "input_item_count")).transpose()?,
                (*tool_count).map(|value| sqlite_u64(value, "tool_count")).transpose()?,
                (*text_input_block_count).map(|value| sqlite_u64(value, "text_input_block_count")).transpose()?,
                (*image_input_block_count).map(|value| sqlite_u64(value, "image_input_block_count")).transpose()?,
                (*file_input_block_count).map(|value| sqlite_u64(value, "file_input_block_count")).transpose()?,
            ],
        ))?),
        WriteCommand::ProviderAttempt {
            attempt_id,
            request_id,
            ordinal,
            status_code,
            started_at,
            ended_at,
            status,
            provider_response_id,
            response_model,
            response_state,
            provider_created_at,
            incomplete_reason,
            error_code,
            transport_error,
            observation_status,
            streaming,
            chunk_count,
            byte_count,
            ttfb_us,
            ttft_us,
            duration_us,
            anomaly_metadata,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO provider_attempts(attempt_id, request_id, ordinal, status_code, started_at, ended_at, status, provider_response_id, response_model, response_state, provider_created_at, incomplete_reason, error_code, transport_error, observation_status, streaming, chunk_count, byte_count, ttfb_us, ttft_us, duration_us, anomaly_metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                attempt_id.to_string(),
                request_id.to_string(),
                sqlite_u64(*ordinal, "attempt_ordinal")?,
                status_code.map(tracepress_core::HttpStatusCode::get),
                started_at,
                ended_at,
                inference_status(*status),
                provider_response_id.as_deref(),
                response_model.as_deref(),
                response_state.as_ref().map(|value| response_state_text(*value)),
                provider_created_at.as_deref(),
                incomplete_reason.as_deref(),
                error_code.as_deref(),
                transport_error.as_deref(),
                observation_status.as_ref().map(|value| observation_status_text(*value)),
                streaming,
                (*chunk_count).map(|value| sqlite_u64(value, "chunk_count")).transpose()?,
                (*byte_count).map(|value| sqlite_u64(value, "byte_count")).transpose()?,
                (*ttfb_us).map(|value| sqlite_u64(value, "ttfb_us")).transpose()?,
                (*ttft_us).map(|value| sqlite_u64(value, "ttft_us")).transpose()?,
                (*duration_us).map(|value| sqlite_u64(value, "duration_us")).transpose()?,
                anomaly_metadata.as_deref(),
            ],
        ))?),
        WriteCommand::ProviderUsage {
            attempt_id,
            input_total,
            input_uncached,
            cache_read,
            cache_write,
            output_total,
            reasoning,
            usage_status,
            raw_usage_json,
            input_cached,
            output_reasoning,
            total,
            normalizer_version,
            anomaly_metadata,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO provider_usage(attempt_id, input_total, input_uncached, cache_read, cache_write, output_total, reasoning, usage_status, raw_usage_json, input_cached, output_reasoning, total, normalizer_version, anomaly_metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                attempt_id.to_string(),
                sqlite_optional(*input_total, "input_total")?,
                sqlite_optional(*input_uncached, "input_uncached")?,
                sqlite_optional(*cache_read, "cache_read")?,
                sqlite_optional(*cache_write, "cache_write")?,
                sqlite_optional(*output_total, "output_total")?,
                sqlite_optional(*reasoning, "reasoning")?,
                usage_status.as_ref().map(|value| usage_status_text(*value)),
                raw_usage_json.as_deref(),
                (*input_cached).map(|value| sqlite_u64(value, "input_cached")).transpose()?,
                (*output_reasoning).map(|value| sqlite_u64(value, "output_reasoning")).transpose()?,
                (*total).map(|value| sqlite_u64(value, "total")).transpose()?,
                (*normalizer_version).map(|value| sqlite_u64(u64::from(value), "normalizer_version")).transpose()?,
                anomaly_metadata.as_deref(),
            ],
        ))?),
        WriteCommand::ContentObject {
            object,
            pin_count,
            created_at,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO content_objects(content_id, raw_bytes, external_ref, byte_length, content_kind, pin_count, created_at) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6)",
            params![object.content_id().to_string(), object.raw_bytes(), sqlite_u64(u64::try_from(object.raw_bytes().len()).map_err(|_error| StorageError::IntegerOverflow { field: "content_byte_length", value: u64::MAX })?, "content_byte_length")?, content_kind(object.kind()), sqlite_u64(*pin_count, "pin_count")?, created_at],
        ))?),
        WriteCommand::ContentOccurrence {
            occurrence,
            observed_at,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO content_occurrences(occurrence_id, content_id, session_id, operation_id, role, observed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![occurrence.occurrence_id().to_string(), occurrence.content_id().to_string(), occurrence.session_id().to_string(), occurrence.operation_id().map(|id| id.to_string()), content_role(occurrence.role()), observed_at],
        ))?),
        WriteCommand::ContentBinding { binding } => committed(sqlite(transaction.execute(
            "INSERT INTO content_bindings(binding_id, session_id, logical_content_id, raw_content_id, rendered_content_id, compressor_version, policy_version, frozen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
            params![binding.binding_id().to_string(), binding.session_id().to_string(), binding.logical_content_id().to_string(), binding.raw_content_id().to_string(), binding.rendered_content_id().to_string(), binding.compressor_version(), binding.policy_version()],
        ))?),
        WriteCommand::CompressionDecision {
            decision_id,
            session_id,
            operation_id,
            input_content_id,
            output_content_id,
            fidelity,
            recoverable,
            compressor,
            compressor_version,
            policy_version,
            raw_bytes,
            output_bytes,
            estimated_raw_tokens,
            estimated_output_tokens,
            target_tokens,
            latency_us,
            feature_schema_version,
            features,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO compression_decisions(decision_id, session_id, operation_id, input_content_id, output_content_id, fidelity, recoverable, compressor, compressor_version, policy_version, raw_bytes, output_bytes, estimated_raw_tokens, estimated_output_tokens, target_tokens, latency_us, feature_schema_version, features) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![decision_id.to_string(), session_id.to_string(), operation_id.to_string(), input_content_id.to_string(), output_content_id.to_string(), fidelity_class(*fidelity), i64::from(*recoverable), compressor, compressor_version, policy_version, sqlite_u64(*raw_bytes, "raw_bytes")?, sqlite_u64(*output_bytes, "output_bytes")?, sqlite_optional(*estimated_raw_tokens, "estimated_raw_tokens")?, sqlite_optional(*estimated_output_tokens, "estimated_output_tokens")?, sqlite_optional(*target_tokens, "target_tokens")?, sqlite_optional(*latency_us, "latency_us")?, feature_schema_version, features.as_deref()],
        ))?),
        WriteCommand::Recovery {
            recovery_id,
            decision_id,
            raw_content_id,
            provenance_content_id,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO recoveries(recovery_id, decision_id, raw_content_id, provenance_content_id) VALUES (?1, ?2, ?3, ?4)",
            params![recovery_id.to_string(), decision_id.to_string(), raw_content_id.to_string(), provenance_content_id.map(|id| id.to_string())],
        ))?),
        WriteCommand::PolicyAssignment {
            policy_assignment_id,
            session_id,
            policy_version,
            assigned_at,
            chosen_action,
            candidate_actions,
            action_probability,
            random_seed,
            feature_vector,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO policy_assignments(policy_assignment_id, session_id, policy_version, assigned_at, chosen_action, candidate_actions, action_probability, random_seed, feature_vector) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![policy_assignment_id.to_string(), session_id.to_string(), policy_version, assigned_at, chosen_action, candidate_actions.as_deref(), action_probability, sqlite_optional(*random_seed, "random_seed")?, feature_vector.as_deref()],
        ))?),
        WriteCommand::Evaluation {
            evaluation_id,
            session_id,
            operation_id,
            provider_cost_microusd,
            input_tokens,
            cache_tokens,
            output_tokens,
            recoveries,
            reruns,
            latency_ms,
            task_success,
            user_correction,
            quality_score,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO evaluations(evaluation_id, session_id, operation_id, provider_cost_microusd, input_tokens, cache_tokens, output_tokens, recoveries, reruns, latency_ms, task_success, user_correction, quality_score) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![evaluation_id.to_string(), session_id.to_string(), operation_id.map(|id| id.to_string()), sqlite_optional(*provider_cost_microusd, "provider_cost_microusd")?, sqlite_optional(*input_tokens, "input_tokens")?, sqlite_optional(*cache_tokens, "cache_tokens")?, sqlite_optional(*output_tokens, "output_tokens")?, sqlite_optional(*recoveries, "recoveries")?, sqlite_optional(*reruns, "reruns")?, sqlite_optional(*latency_ms, "latency_ms")?, task_success.map(i64::from), user_correction.map(i64::from), quality_score],
        ))?),
        WriteCommand::Event {
            event_id,
            session_id,
            operation_id,
            timestamp,
            event_type,
            payload,
            schema_version,
        } => {
            let _rows = sqlite(transaction.execute(
                "INSERT INTO events(event_id, session_id, operation_id, timestamp, event_type, payload, schema_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![event_id.to_string(), session_id.map(|id| id.to_string()), operation_id.map(|id| id.to_string()), timestamp, event_type, payload.as_ref(), schema_version],
            ))?;
            let sequence = u64::try_from(transaction.last_insert_rowid()).map_err(|_error| {
                StorageError::IntegerOverflow {
                    field: "event_sequence",
                    value: u64::MAX,
                }
            })?;
            Ok(WriteReceipt::EventAppended { sequence })
        }
        WriteCommand::ContextSnapshot {
            snapshot_id,
            session_id,
            provider_request_id,
            inference_operation_id,
            analysis_version,
            status,
            started_at_us,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO context_snapshots(snapshot_id, session_id, provider_request_id, inference_operation_id, analysis_version, status, started_at_us) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                snapshot_id.to_string(),
                session_id.to_string(),
                provider_request_id.to_string(),
                inference_operation_id.to_string(),
                sqlite_u64(u64::from(*analysis_version), "analysis_version")?,
                context_analysis_status_text(*status),
                sqlite_u64(*started_at_us, "started_at_us")?,
            ],
        ))?),
        WriteCommand::ContextSnapshotOutcome {
            snapshot_id,
            status,
            completed_at_us,
            request_content_hash,
            explicit_block_count,
            analyzed_bytes,
            skipped_bytes,
            explicit_request_complete,
            uses_previous_response,
            uses_conversation_state,
            uses_item_references,
            uses_prompt_reference,
            uses_external_files,
            uses_external_images,
            contains_opaque_items,
            logical_context_status,
            duplicate_key_detected,
            reference_resolved_locally,
            correlation_status,
        } => committed(sqlite(transaction.execute(
            "UPDATE context_snapshots SET status = ?2, completed_at_us = ?3, request_content_hash = ?4, explicit_block_count = ?5, analyzed_bytes = ?6, skipped_bytes = ?7, explicit_request_complete = ?8, uses_previous_response = ?9, uses_conversation_state = ?10, uses_item_references = ?11, uses_prompt_reference = ?12, uses_external_files = ?13, uses_external_images = ?14, contains_opaque_items = ?15, logical_context_status = ?16, duplicate_key_detected = ?17, reference_resolved_locally = ?18, correlation_status = ?19 WHERE snapshot_id = ?1",
            params![
                snapshot_id.to_string(),
                context_analysis_status_text(*status),
                sqlite_optional(*completed_at_us, "completed_at_us")?,
                request_content_hash.as_deref(),
                sqlite_optional(*explicit_block_count, "explicit_block_count")?,
                sqlite_optional(*analyzed_bytes, "analyzed_bytes")?,
                sqlite_optional(*skipped_bytes, "skipped_bytes")?,
                explicit_request_complete,
                uses_previous_response,
                uses_conversation_state,
                uses_item_references,
                uses_prompt_reference,
                uses_external_files,
                uses_external_images,
                contains_opaque_items,
                logical_context_status.as_ref().map(|value| logical_context_status_text(*value)),
                duplicate_key_detected,
                reference_resolved_locally,
                correlation_status.as_ref().map(|value| context_correlation_status_text(*value)),
            ],
        ))?),
        WriteCommand::ContextBlockOccurrence {
            block_occurrence_id,
            snapshot_id,
            ordinal,
            parent_block_occurrence_id,
            kind,
            role,
            origin,
            semantic_path,
            semantic_path_truncated,
            semantic_path_hash,
            raw_value_start,
            raw_value_end,
            locator_occurrence,
            raw_bytes,
            exact_fingerprint,
            semantic_fingerprint,
            fingerprint_version,
            estimated_tokens,
            estimator,
            estimator_version,
            estimator_encoding,
            estimate_confidence,
            detected_kind,
            detector_confidence,
            detector_version,
            tool_call_id,
            tool_name,
            tool_name_truncated,
            tool_name_hash,
            line_count,
            max_line_length,
            duplicate_line_ratio,
            unique_line_ratio,
            json_item_count,
            json_depth,
            error_line_density,
            warning_line_density,
            repetition_score,
            opportunity_signals,
            candidate_estimated_tokens,
        } => {
            let signals = opportunity_signals
                .as_deref()
                .map(opportunity_signals_text);
            committed(sqlite(transaction.execute(
                "INSERT INTO context_block_occurrences(block_occurrence_id, snapshot_id, ordinal, parent_block_occurrence_id, kind, role, origin, semantic_path, semantic_path_truncated, semantic_path_hash, raw_value_start, raw_value_end, locator_occurrence, raw_bytes, exact_fingerprint, semantic_fingerprint, fingerprint_version, estimated_tokens, estimator, estimator_version, estimator_encoding, estimate_confidence, detected_kind, detector_confidence, detector_version, tool_call_id, tool_name, tool_name_truncated, tool_name_hash, line_count, max_line_length, duplicate_line_ratio, unique_line_ratio, json_item_count, json_depth, error_line_density, warning_line_density, repetition_score, opportunity_signals, candidate_estimated_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40)",
                params![
                    block_occurrence_id.to_string(),
                    snapshot_id.to_string(),
                    sqlite_u64(*ordinal, "block_ordinal")?,
                    parent_block_occurrence_id.map(|id| id.to_string()),
                    context_block_kind_text(*kind),
                    context_role_text(*role),
                    context_origin_text(*origin),
                    semantic_path.as_deref(),
                    semantic_path_truncated,
                    semantic_path_hash.as_deref(),
                    sqlite_u64(*raw_value_start, "raw_value_start")?,
                    sqlite_u64(*raw_value_end, "raw_value_end")?,
                    sqlite_u64(*locator_occurrence, "locator_occurrence")?,
                    sqlite_u64(*raw_bytes, "block_raw_bytes")?,
                    exact_fingerprint.as_deref(),
                    semantic_fingerprint.as_deref(),
                    (*fingerprint_version).map(|value| sqlite_u64(u64::from(value), "fingerprint_version")).transpose()?,
                    sqlite_optional(*estimated_tokens, "estimated_tokens")?,
                    estimator.as_deref(),
                    (*estimator_version).map(|value| sqlite_u64(u64::from(value), "estimator_version")).transpose()?,
                    estimator_encoding.as_deref(),
                    estimate_confidence.as_ref().map(|value| estimate_confidence_text(*value)),
                    detected_kind.as_ref().map(|value| detected_content_kind_text(*value)),
                    detector_confidence,
                    (*detector_version).map(|value| sqlite_u64(u64::from(value), "detector_version")).transpose()?,
                    tool_call_id.as_deref(),
                    tool_name.as_deref(),
                    tool_name_truncated,
                    tool_name_hash.as_deref(),
                    sqlite_optional(*line_count, "line_count")?,
                    sqlite_optional(*max_line_length, "max_line_length")?,
                    duplicate_line_ratio,
                    unique_line_ratio,
                    sqlite_optional(*json_item_count, "json_item_count")?,
                    sqlite_optional(*json_depth, "json_depth")?,
                    error_line_density,
                    warning_line_density,
                    repetition_score,
                    signals,
                    sqlite_optional(*candidate_estimated_tokens, "candidate_estimated_tokens")?,
                ],
            ))?)
        }
        WriteCommand::ContextAnalysisMetrics {
            snapshot_id,
            explicit_bytes,
            estimated_tokens,
            estimated_tool_definition_share,
            estimated_tool_result_share,
            estimated_human_text_share,
            estimated_assistant_history_share,
            estimated_unique_content_share,
            estimated_repeated_content_share,
            tool_count,
            schema_bytes,
            estimated_schema_tokens,
            largest_tool_schema,
            repeated_schema_tokens,
            stable_explicit_prefix_estimate,
            estimator,
            estimator_version,
            estimate_confidence,
            opportunity_signals,
        } => {
            let signals = opportunity_signals
                .as_deref()
                .map(opportunity_signals_text);
            let kinds = estimated_tokens.by_kind();
            let roles = estimated_tokens.by_role();
            let origins = estimated_tokens.by_origin();
            committed(sqlite(transaction.execute(
                "INSERT INTO context_analysis_metrics(snapshot_id, explicit_bytes, estimated_tokens_kind_instructions, estimated_tokens_kind_message, estimated_tokens_kind_text, estimated_tokens_kind_image_reference, estimated_tokens_kind_file_reference, estimated_tokens_kind_tool_definition, estimated_tokens_kind_tool_call, estimated_tokens_kind_tool_result, estimated_tokens_kind_item_reference, estimated_tokens_kind_prompt_reference, estimated_tokens_kind_provider_state_reference, estimated_tokens_kind_assistant_history, estimated_tokens_kind_opaque_reasoning, estimated_tokens_kind_opaque, estimated_tokens_kind_unknown, estimated_tokens_role_system, estimated_tokens_role_developer, estimated_tokens_role_user, estimated_tokens_role_assistant, estimated_tokens_role_tool, estimated_tokens_role_unknown, estimated_tokens_origin_human_authored, estimated_tokens_origin_agent_generated, estimated_tokens_origin_tool_generated, estimated_tokens_origin_tool_schema, estimated_tokens_origin_provider_managed, estimated_tokens_origin_external_reference, estimated_tokens_origin_tracepress_generated, estimated_tokens_origin_unknown, estimated_tool_definition_share, estimated_tool_result_share, estimated_human_text_share, estimated_assistant_history_share, estimated_unique_content_share, estimated_repeated_content_share, tool_count, schema_bytes, estimated_schema_tokens, largest_tool_schema, repeated_schema_tokens, stable_explicit_prefix_estimate, estimator, estimator_version, estimate_confidence, opportunity_signals) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45, ?46, ?47)",
                params![
                    snapshot_id.to_string(),
                    sqlite_optional(*explicit_bytes, "explicit_bytes")?,
                    sqlite_optional(kinds.get(ContextBlockKind::Instructions), "estimated_tokens_kind_instructions")?,
                    sqlite_optional(kinds.get(ContextBlockKind::Message), "estimated_tokens_kind_message")?,
                    sqlite_optional(kinds.get(ContextBlockKind::Text), "estimated_tokens_kind_text")?,
                    sqlite_optional(kinds.get(ContextBlockKind::ImageReference), "estimated_tokens_kind_image_reference")?,
                    sqlite_optional(kinds.get(ContextBlockKind::FileReference), "estimated_tokens_kind_file_reference")?,
                    sqlite_optional(kinds.get(ContextBlockKind::ToolDefinition), "estimated_tokens_kind_tool_definition")?,
                    sqlite_optional(kinds.get(ContextBlockKind::ToolCall), "estimated_tokens_kind_tool_call")?,
                    sqlite_optional(kinds.get(ContextBlockKind::ToolResult), "estimated_tokens_kind_tool_result")?,
                    sqlite_optional(kinds.get(ContextBlockKind::ItemReference), "estimated_tokens_kind_item_reference")?,
                    sqlite_optional(kinds.get(ContextBlockKind::PromptReference), "estimated_tokens_kind_prompt_reference")?,
                    sqlite_optional(kinds.get(ContextBlockKind::ProviderStateReference), "estimated_tokens_kind_provider_state_reference")?,
                    sqlite_optional(kinds.get(ContextBlockKind::AssistantHistory), "estimated_tokens_kind_assistant_history")?,
                    sqlite_optional(kinds.get(ContextBlockKind::OpaqueReasoning), "estimated_tokens_kind_opaque_reasoning")?,
                    sqlite_optional(kinds.get(ContextBlockKind::Opaque), "estimated_tokens_kind_opaque")?,
                    sqlite_optional(kinds.get(ContextBlockKind::Unknown), "estimated_tokens_kind_unknown")?,
                    sqlite_optional(roles.get(ContextRole::System), "estimated_tokens_role_system")?,
                    sqlite_optional(roles.get(ContextRole::Developer), "estimated_tokens_role_developer")?,
                    sqlite_optional(roles.get(ContextRole::User), "estimated_tokens_role_user")?,
                    sqlite_optional(roles.get(ContextRole::Assistant), "estimated_tokens_role_assistant")?,
                    sqlite_optional(roles.get(ContextRole::Tool), "estimated_tokens_role_tool")?,
                    sqlite_optional(roles.get(ContextRole::Unknown), "estimated_tokens_role_unknown")?,
                    sqlite_optional(origins.get(ContextOrigin::HumanAuthored), "estimated_tokens_origin_human_authored")?,
                    sqlite_optional(origins.get(ContextOrigin::AgentGenerated), "estimated_tokens_origin_agent_generated")?,
                    sqlite_optional(origins.get(ContextOrigin::ToolGenerated), "estimated_tokens_origin_tool_generated")?,
                    sqlite_optional(origins.get(ContextOrigin::ToolSchema), "estimated_tokens_origin_tool_schema")?,
                    sqlite_optional(origins.get(ContextOrigin::ProviderManaged), "estimated_tokens_origin_provider_managed")?,
                    sqlite_optional(origins.get(ContextOrigin::ExternalReference), "estimated_tokens_origin_external_reference")?,
                    sqlite_optional(origins.get(ContextOrigin::TracepressGenerated), "estimated_tokens_origin_tracepress_generated")?,
                    sqlite_optional(origins.get(ContextOrigin::Unknown), "estimated_tokens_origin_unknown")?,
                    estimated_tool_definition_share,
                    estimated_tool_result_share,
                    estimated_human_text_share,
                    estimated_assistant_history_share,
                    estimated_unique_content_share,
                    estimated_repeated_content_share,
                    sqlite_optional(*tool_count, "metrics_tool_count")?,
                    sqlite_optional(*schema_bytes, "schema_bytes")?,
                    sqlite_optional(*estimated_schema_tokens, "estimated_schema_tokens")?,
                    sqlite_optional(*largest_tool_schema, "largest_tool_schema")?,
                    sqlite_optional(*repeated_schema_tokens, "repeated_schema_tokens")?,
                    sqlite_optional(*stable_explicit_prefix_estimate, "stable_explicit_prefix_estimate")?,
                    estimator.as_deref(),
                    (*estimator_version).map(|value| sqlite_u64(u64::from(value), "metrics_estimator_version")).transpose()?,
                    estimate_confidence.as_ref().map(|value| estimate_confidence_text(*value)),
                    signals,
                ],
            ))?)
        }
        WriteCommand::ContextDelta {
            current_snapshot_id,
            previous_snapshot_id,
            repeated_blocks,
            new_blocks,
            changed_blocks,
            removed_blocks,
            repeated_estimated_tokens,
            new_estimated_tokens,
            common_prefix_blocks,
            common_prefix_estimated_tokens,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO context_deltas(current_snapshot_id, previous_snapshot_id, repeated_blocks, new_blocks, changed_blocks, removed_blocks, repeated_estimated_tokens, new_estimated_tokens, common_prefix_blocks, common_prefix_estimated_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                current_snapshot_id.to_string(),
                previous_snapshot_id.to_string(),
                sqlite_optional(*repeated_blocks, "repeated_blocks")?,
                sqlite_optional(*new_blocks, "new_blocks")?,
                sqlite_optional(*changed_blocks, "changed_blocks")?,
                sqlite_optional(*removed_blocks, "removed_blocks")?,
                sqlite_optional(*repeated_estimated_tokens, "repeated_estimated_tokens")?,
                sqlite_optional(*new_estimated_tokens, "new_estimated_tokens")?,
                sqlite_optional(*common_prefix_blocks, "common_prefix_blocks")?,
                sqlite_optional(*common_prefix_estimated_tokens, "common_prefix_estimated_tokens")?,
            ],
        ))?),
        WriteCommand::TokenReconciliation {
            snapshot_id,
            attempt_id,
            visible_estimated_tokens,
            provider_input_tokens,
            residual_tokens,
            comparability,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO token_reconciliations(snapshot_id, attempt_id, visible_estimated_tokens, provider_input_tokens, residual_tokens, comparability) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot_id.to_string(),
                attempt_id.map(|id| id.to_string()),
                sqlite_optional(*visible_estimated_tokens, "visible_estimated_tokens")?,
                sqlite_optional(*provider_input_tokens, "provider_input_tokens")?,
                residual_tokens,
                reconciliation_status_text(*comparability),
            ],
        ))?),
    }
}

fn committed(rows_changed: usize) -> Result<WriteReceipt, StorageError> {
    let rows_changed =
        u64::try_from(rows_changed).map_err(|_error| StorageError::IntegerOverflow {
            field: "rows_changed",
            value: u64::MAX,
        })?;
    Ok(WriteReceipt::Committed { rows_changed })
}
