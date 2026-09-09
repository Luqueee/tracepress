#![allow(
    clippy::redundant_pub_crate,
    reason = "the writer sibling module consumes these transaction functions"
)]

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::{
    StorageError, WriteBatch, WriteCommand, WriteReceipt,
    encode::{
        causal_relationship, content_kind, content_role, fidelity_class, inference_status,
        operation_kind, operation_status, request_method, request_route, session_state, sqlite,
        sqlite_optional, sqlite_u64, usage_status_text,
    },
};

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
) -> Result<WriteReceipt, StorageError> {
    let transaction = sqlite(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let rows = sqlite(transaction.execute(
        "UPDATE sessions SET state = ?1, ended_at = COALESCE(ended_at, ?2) WHERE state IN (?3, ?4)",
        params![
            session_state(tracepress_core::SessionState::Stale),
            recovered_at,
            session_state(tracepress_core::SessionState::Active),
            session_state(tracepress_core::SessionState::Closing)
        ],
    ))?;
    sqlite(transaction.commit())?;
    committed(rows)
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
        } => committed(sqlite(transaction.execute(
            "INSERT INTO provider_requests(request_id, operation_id, route, method, request_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![metadata.request_id().to_string(), operation_id.to_string(), request_route(metadata.route()), request_method(metadata.method()), sqlite_u64(metadata.request_bytes(), "request_bytes")?],
        ))?),
        WriteCommand::ProviderAttempt {
            attempt_id,
            request_id,
            ordinal,
            status_code,
            started_at,
            ended_at,
            status,
        } => committed(sqlite(transaction.execute(
            "INSERT INTO provider_attempts(attempt_id, request_id, ordinal, status_code, started_at, ended_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![attempt_id.to_string(), request_id.to_string(), sqlite_u64(*ordinal, "attempt_ordinal")?, status_code.map(tracepress_core::HttpStatusCode::get), started_at, ended_at, inference_status(*status)],
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
        } => committed(sqlite(transaction.execute(
            "INSERT INTO provider_usage(attempt_id, input_total, input_uncached, cache_read, cache_write, output_total, reasoning, usage_status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![attempt_id.to_string(), sqlite_optional(*input_total, "input_total")?, sqlite_optional(*input_uncached, "input_uncached")?, sqlite_optional(*cache_read, "cache_read")?, sqlite_optional(*cache_write, "cache_write")?, sqlite_optional(*output_total, "output_total")?, sqlite_optional(*reasoning, "reasoning")?, usage_status.map(usage_status_text)],
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
