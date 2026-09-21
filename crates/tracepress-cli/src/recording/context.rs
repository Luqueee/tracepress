//! Deferred context persistence, metrics, and snapshot reconciliation.

use super::*;

/// Owns every potentially slow context IPC operation.
///
/// The provider recorder never waits on this worker: it only admits a compact job with the
/// provider receipt identities already returned by `RecordProviderObservation`.
#[derive(Debug)]
pub(super) struct ContextIngestionWorker {
    pub(super) config: Config,
    pub(super) session_id: SessionId,
    pub(super) context_analysis_limits: ContextAnalysisLimits,
    pub(super) previous_context: Option<PreviousContextSnapshot>,
    pub(super) counters: Arc<ContextCounters>,
    pub(super) shadow_sender: Option<tokio::sync::mpsc::Sender<ShadowJob>>,
    pub(super) shadow_budget: Arc<ShadowByteBudget>,
    pub(super) shadow_counters: Arc<ShadowCounters>,
}

impl ContextIngestionWorker {
    pub(super) async fn run(
        mut self,
        mut jobs: tokio::sync::mpsc::Receiver<ContextIngestionJob>,
    ) -> Result<(), String> {
        while let Some(job) = jobs.recv().await {
            self.record_context(job).await;
        }
        self.flush_drops().await;
        Ok(())
    }

    async fn flush_drops(&self) {
        flush_context_drops(&self.config, self.session_id, &self.counters).await;
    }

    async fn begin_context(
        &self,
        provider_request_id: RequestId,
        inference_operation_id: OperationId,
    ) -> Result<(ContextSnapshotId, u64), ContextAnalysisDropReason> {
        let started_at_us = current_timestamp_us()
            .map_err(|_error| ContextAnalysisDropReason::CorrelationDegraded)?;
        let response = control(
            &self.config,
            ControlRequest::BeginContextAnalysis {
                session_id: self.session_id,
                provider_request_id,
                inference_operation_id,
                analysis_version: tracepress_context::CONTEXT_ANALYSIS_VERSION,
                started_at_us,
            },
        )
        .await;
        let response = response.map_err(|_error| ContextAnalysisDropReason::CorrelationDegraded)?;
        match response {
            ControlResponse::Ok {
                context_snapshot_id: Some(snapshot_id),
                ..
            } => Ok((snapshot_id, started_at_us)),
            ControlResponse::Error { .. } | ControlResponse::Context { .. } => {
                Err(ContextAnalysisDropReason::CorrelationDegraded)
            }
            _ => Err(ContextAnalysisDropReason::Unsupported),
        }
    }

    async fn abort_context(
        &self,
        snapshot_id: ContextSnapshotId,
        reason: ContextAnalysisDropReason,
    ) {
        let Ok(completed_at_us) = current_timestamp_us() else {
            return;
        };
        let _ = control(
            &self.config,
            ControlRequest::AbortContextAnalysis {
                snapshot_id,
                reason,
                completed_at_us,
            },
        )
        .await;
    }

    async fn append_batch(
        &self,
        input: ContextAppendBatchInput,
    ) -> Result<ContextAppendReceipt, ContextAnalysisDropReason> {
        let ContextAppendBatchInput {
            snapshot_id,
            sequence,
            blocks,
        } = input;
        let response = control(
            &self.config,
            ControlRequest::AppendContextBlocks {
                snapshot_id,
                sequence,
                blocks,
            },
        )
        .await
        .map_err(|_error| ContextAnalysisDropReason::CorrelationDegraded)?;
        match response {
            ControlResponse::Ok {
                context_append_receipt: Some(receipt),
                ..
            } => Ok(receipt),
            ControlResponse::Error { .. } | ControlResponse::Context { .. } => {
                Err(ContextAnalysisDropReason::CorrelationDegraded)
            }
            _ => Err(ContextAnalysisDropReason::Unsupported),
        }
    }

    async fn append_context_blocks(
        &self,
        input: ContextAppendInput<'_>,
    ) -> Result<(ContextAnalysisStatus, u64), ContextAnalysisDropReason> {
        let ContextAppendInput {
            snapshot_id,
            analysis,
            mut status,
        } = input;
        let mut sequence = 0_u32;
        let max_batches =
            u32::try_from(self.context_analysis_limits.max_batches.get()).unwrap_or(u32::MAX);
        // `IpcRequest` serializes its opaque body as a JSON byte array; four frame bytes per
        // logical byte is the conservative bound that keeps the enclosing request under 64 KiB.
        let body_limit = usize::try_from(BODY_BYTES / 4).unwrap_or(0);
        let total_blocks = u64::try_from(analysis.blocks.len()).unwrap_or(u64::MAX);
        let mut batch = Vec::new();
        let mut accepted_block_count = 0_u64;
        let mut capacity_exhausted = false;

        for block in analysis.blocks.iter().cloned() {
            if capacity_exhausted || sequence >= max_batches {
                break;
            }
            let mut candidate = batch.clone();
            candidate.push(block.clone());
            let request = ControlRequest::AppendContextBlocks {
                snapshot_id,
                sequence,
                blocks: candidate,
            };
            let Ok(encoded) = serde_json::to_vec(&request) else {
                break;
            };
            if encoded.len() <= body_limit {
                batch.push(block);
                continue;
            }
            if batch.is_empty() {
                // A singleton that cannot fit is an explicit resource-limited prefix.
                break;
            }
            let outgoing = std::mem::take(&mut batch);
            let receipt = self
                .append_batch(ContextAppendBatchInput {
                    snapshot_id,
                    sequence,
                    blocks: outgoing,
                })
                .await?;
            accepted_block_count = receipt.accepted_block_count;
            sequence = receipt.next_sequence;
            capacity_exhausted = receipt.capacity.is_exhausted();
            if capacity_exhausted {
                break;
            }
            let singleton = vec![block.clone()];
            let singleton_request = ControlRequest::AppendContextBlocks {
                snapshot_id,
                sequence,
                blocks: singleton,
            };
            let Ok(singleton_encoded) = serde_json::to_vec(&singleton_request) else {
                break;
            };
            if singleton_encoded.len() > body_limit {
                break;
            }
            batch.push(block);
        }
        if !batch.is_empty() && !capacity_exhausted && sequence < max_batches {
            let receipt = self
                .append_batch(ContextAppendBatchInput {
                    snapshot_id,
                    sequence,
                    blocks: batch,
                })
                .await?;
            accepted_block_count = receipt.accepted_block_count;
        }
        if accepted_block_count < total_blocks {
            status = ContextAnalysisStatus::ResourceLimit;
        }
        Ok((status, accepted_block_count))
    }
    async fn record_context(&mut self, job: ContextIngestionJob) {
        let ContextIngestionJob {
            forward,
            provider_request_id,
            attempt_id,
            inference_operation_id,
            analysis,
            shadow_body,
            analysis_permit,
            provider_input_tokens,
            provider_usage_comparable,
            correlation,
        } = job;
        let _analysis_permit = analysis_permit;
        let (snapshot_id, started_at_us) = match self
            .begin_context(provider_request_id, inference_operation_id)
            .await
        {
            Ok(value) => value,
            Err(reason) => {
                self.counters.dropped(forward, reason);
                return;
            }
        };
        self.counters.remember_snapshot(forward, snapshot_id);
        let initial_status = if matches!(correlation, CorrelationStatus::Correlated) {
            analysis.status
        } else {
            ContextAnalysisStatus::CorrelationDegraded
        };
        let (status, accepted_block_count) = match self
            .append_context_blocks(ContextAppendInput {
                snapshot_id,
                analysis: &analysis,
                status: initial_status,
            })
            .await
        {
            Ok(value) => value,
            Err(reason) => {
                self.abort_context(snapshot_id, reason).await;
                self.counters.dropped(forward, reason);
                return;
            }
        };
        let finalized = self
            .finalize_context(ContextFinalizationInput {
                forward,
                snapshot_id,
                started_at_us,
                attempt_id,
                analysis: &analysis,
                provider_input_tokens,
                provider_usage_comparable,
                correlation,
                status,
                accepted_block_count,
            })
            .await;
        if finalized {
            self.enqueue_shadow(snapshot_id, analysis, accepted_block_count, shadow_body);
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the handoff retains explicit snapshot association"
    )]
    fn enqueue_shadow(
        &self,
        snapshot_id: ContextSnapshotId,
        analysis: ContextAnalysisResult,
        accepted_block_count: u64,
        body: Option<ShadowAnalysisBody>,
    ) {
        let (Some(sender), Some(body)) = (&self.shadow_sender, body) else {
            return;
        };
        let bytes = u64::try_from(body.len()).unwrap_or(u64::MAX);
        let Some(byte_permit) = self.shadow_budget.try_acquire(bytes) else {
            self.shadow_counters.record_job_drop();
            self.shadow_counters
                .record_drop(ShadowDropReason::ByteBudget);
            return;
        };
        let job = ShadowJob {
            snapshot_id,
            analysis,
            accepted_block_count,
            body,
            _byte_permit: byte_permit,
        };
        match sender.try_send(job) {
            Ok(()) => self.shadow_counters.record_job_admitted(),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_job)) => {
                self.shadow_counters.record_job_drop();
                self.shadow_counters
                    .record_drop(ShadowDropReason::QueueFull);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_job)) => {
                self.shadow_counters.record_job_drop();
                self.shadow_counters
                    .record_drop(ShadowDropReason::WorkerClosed);
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the finalization boundary assembles all durable context evidence atomically"
    )]
    fn build_context_finalize(
        &self,
        input: &ContextFinalizationInput<'_>,
    ) -> Option<(
        Vec<ContextBlockSummary>,
        ContextAnalysisFinalize,
        ContextCoverage,
    )> {
        let analysis = input.analysis;
        let accepted_len = usize::try_from(input.accepted_block_count)
            .unwrap_or(analysis.blocks.len())
            .min(analysis.blocks.len());
        let accepted_blocks = &analysis.blocks[..accepted_len];
        let current_blocks = context_block_summaries(accepted_blocks);
        let delta = self.previous_context.as_ref().map(|previous| {
            compute_context_delta(ContextDeltaRequest {
                previous_snapshot_id: previous.id,
                current_snapshot_id: input.snapshot_id,
                previous_analysis_status: previous.status,
                current_analysis_status: input.status,
                previous: &previous.blocks,
                current: &current_blocks,
                limits: self.context_analysis_limits,
            })
        });
        let (mut metrics, visible_estimated_tokens) = context_metrics(
            accepted_blocks,
            input.status,
            delta
                .as_ref()
                .and_then(|value| value.common_prefix_estimated_tokens),
        );
        let unknown_block_count = accepted_blocks
            .iter()
            .filter(|block| block.kind == ContextBlockKind::Unknown)
            .count();
        metrics.unknown_block_count = Some(u64::try_from(unknown_block_count).unwrap_or(u64::MAX));
        metrics.semantic_coverage_basis_points = if input.status != ContextAnalysisStatus::Complete
            || accepted_len != analysis.blocks.len()
            || accepted_blocks.is_empty()
        {
            None
        } else {
            #[allow(
                clippy::arithmetic_side_effects,
                reason = "the bounded block count makes this basis-point projection intentional"
            )]
            let basis_points = accepted_blocks
                .len()
                .saturating_sub(unknown_block_count)
                .saturating_mul(10_000)
                / accepted_blocks.len();
            u16::try_from(basis_points).ok()
        };
        let token_eligible = accepted_blocks
            .iter()
            .filter(|block| {
                block.token_estimation_applicability == MeasurementApplicability::Eligible
            })
            .count();
        let token_observed = accepted_blocks
            .iter()
            .filter(|block| block.token_estimate.is_some())
            .count();
        let semantic_eligible = accepted_blocks
            .iter()
            .filter(|block| block.detection_applicability == MeasurementApplicability::Eligible)
            .count();
        let semantic_observed = accepted_blocks
            .iter()
            .filter(|block| block.detection_result.is_some())
            .count();
        let coverage = ContextCoverage {
            token_estimation_eligible: u64::try_from(token_eligible).unwrap_or(u64::MAX),
            token_estimation_observed: u64::try_from(token_observed).unwrap_or(u64::MAX),
            semantic_detection_eligible: u64::try_from(semantic_eligible).unwrap_or(u64::MAX),
            semantic_detection_observed: u64::try_from(semantic_observed).unwrap_or(u64::MAX),
            correlation_eligible: 1,
            correlation_correlated: u64::from(matches!(
                input.correlation,
                CorrelationStatus::Correlated
            )),
        };
        let correlation_status = if matches!(input.correlation, CorrelationStatus::Correlated) {
            ContextCorrelationStatusWire::Correlated
        } else {
            ContextCorrelationStatusWire::Degraded
        };
        let reconciliation = TokenReconciliation::reconcile(
            input.snapshot_id,
            analysis.visibility,
            visible_estimated_tokens,
            input.provider_input_tokens,
            input.provider_usage_comparable
                && matches!(
                    input.status,
                    ContextAnalysisStatus::Complete
                        | ContextAnalysisStatus::Partial
                        | ContextAnalysisStatus::ResourceLimit
                ),
        );
        let finalize = ContextAnalysisFinalize::builder(
            input.snapshot_id,
            input.status,
            current_timestamp_us().unwrap_or(input.started_at_us),
        )
        .request_content_hash(Some(analysis.request_content_hash))
        .analysis_content_hash(Some(analysis.analysis_content_hash))
        .explicit_block_count(Some(input.accepted_block_count))
        .analyzed_bytes(Some(analysis.analyzed_bytes))
        .skipped_bytes(Some(analysis.skipped_bytes))
        .visibility(analysis.visibility)
        .duplicate_key_detected(Some(analysis.duplicate_key_detected))
        .reference_resolved_locally(Some(analysis.visibility_facts.reference_resolved_locally))
        .correlation_status(correlation_status)
        .metrics(metrics)
        .delta(delta)
        .reconciliation(reconciliation)
        .attempt_id(Some(input.attempt_id))
        .build()
        .ok()?;
        Some((current_blocks, finalize, coverage))
    }
    async fn finalize_context(&mut self, input: ContextFinalizationInput<'_>) -> bool {
        let forward = input.forward;
        let snapshot_id = input.snapshot_id;
        let status = input.status;
        let Some((current_blocks, finalize, coverage)) = self.build_context_finalize(&input) else {
            self.abort_context(snapshot_id, ContextAnalysisDropReason::CorrelationDegraded)
                .await;
            self.counters
                .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
            return false;
        };
        let finalized = control(
            &self.config,
            ControlRequest::FinalizeContextAnalysis {
                summary: Box::new(finalize),
            },
        )
        .await;
        match finalized {
            Ok(ControlResponse::Ok { .. }) => {
                self.counters.observe_coverage(coverage);
                match status {
                    ContextAnalysisStatus::Complete => self.counters.complete(forward),
                    ContextAnalysisStatus::Partial => self.counters.partial(forward),
                    ContextAnalysisStatus::ResourceLimit => self.counters.partial_with_drop_reason(
                        forward,
                        ContextAnalysisDropReason::ResourceLimit,
                    ),
                    ContextAnalysisStatus::Malformed => self
                        .counters
                        .partial_with_drop_reason(forward, ContextAnalysisDropReason::Malformed),
                    ContextAnalysisStatus::ObserverBackpressure => {
                        self.counters.partial_with_drop_reason(
                            forward,
                            ContextAnalysisDropReason::ObserverBackpressure,
                        );
                    }
                    ContextAnalysisStatus::CorrelationDegraded => {
                        self.counters.partial_with_drop_reason(
                            forward,
                            ContextAnalysisDropReason::CorrelationDegraded,
                        );
                    }
                    ContextAnalysisStatus::Cancelled => self
                        .counters
                        .partial_with_drop_reason(forward, ContextAnalysisDropReason::Cancelled),
                    _ => self
                        .counters
                        .partial_with_drop_reason(forward, ContextAnalysisDropReason::Unsupported),
                }
                self.previous_context = Some(PreviousContextSnapshot {
                    id: snapshot_id,
                    blocks: current_blocks,
                    status,
                });
                true
            }
            Ok(
                ControlResponse::Error { .. }
                | ControlResponse::Context { .. }
                | ControlResponse::ContextStatus { .. },
            )
            | Err(_) => {
                self.abort_context(snapshot_id, ContextAnalysisDropReason::CorrelationDegraded)
                    .await;
                self.counters
                    .dropped(forward, ContextAnalysisDropReason::CorrelationDegraded);
                false
            }
        }
    }
}
pub(crate) async fn flush_context_drops(
    config: &Config,
    session_id: SessionId,
    counters: &ContextCounters,
) {
    let reasons = [
        ContextAnalysisDropReason::ObserverBackpressure,
        ContextAnalysisDropReason::DeferredBacklogCapacity,
        ContextAnalysisDropReason::ResourceLimit,
        ContextAnalysisDropReason::Malformed,
        ContextAnalysisDropReason::CorrelationDegraded,
        ContextAnalysisDropReason::Unsupported,
        ContextAnalysisDropReason::Cancelled,
    ];
    let Ok(observed_at_us) = current_timestamp_us() else {
        return;
    };
    for reason in reasons {
        let dropped = counters.take_pending_drop_count(reason);
        if dropped == 0 {
            continue;
        }
        let _ = control(
            config,
            ControlRequest::RecordContextAnalysisDropped {
                session_id,
                reason,
                dropped,
                observed_at_us,
            },
        )
        .await;
    }
}

pub(super) type ObservationRecordParts = (
    ObservationRecord,
    Option<ContextAnalysisOutcome>,
    Option<ShadowAnalysisBody>,
    Option<AnalysisOutputPermit>,
    Option<u64>,
    bool,
);

pub(super) fn observation_record(
    semantic: SemanticRecord,
    ended_at: String,
    correlation: CorrelationStatus,
) -> Option<ObservationRecordParts> {
    let SemanticRecord {
        pending,
        response,
        context,
        shadow_body,
        status_code,
        transport_failure,
        analysis_permit,
    } = semantic;
    // The parser always measures its bounded input; an unmeasured body is never invented.
    // A decoder rejected by the bounded analysis capacity did not present bytes to the semantic
    // parser. The provider row still keeps the exact accepted wire count, while the semantic
    // request length remains NULL in `RequestObservation`.
    let request_bytes = pending
        .request
        .request_bytes
        .or(pending.request.wire_bytes)?;
    let streaming = pending.request.stream;
    // An unobserved or unrepresentable status stays unknown rather than fabricated.
    let status_code = status_code.and_then(|value| HttpStatusCode::new(value).ok());
    // An errored upstream status is terminal evidence by itself: the exchange is over even when
    // nothing interpreted the body, which is what an upstream answering a media type the
    // observer does not read leaves behind. Closing the attempt here keeps the timestamp a real
    // local measurement instead of one the daemon would have to invent, and keeps the daemon
    // from persisting an errored attempt whose `ended_at` is still NULL.
    let errored_upstream = status_code.is_some_and(|code| !(200..300).contains(&code.get()));
    let terminal = response.is_some() || transport_failure.is_some() || errored_upstream;
    let provider_input_tokens = response
        .as_ref()
        .and_then(|value| value.normalized_usage.as_ref())
        .and_then(|usage| usage.input_total);
    let provider_usage_comparable = response.as_ref().is_some_and(|value| {
        value.usage_status == tracepress_provider::UsageStatus::Final
            && value
                .normalized_usage
                .as_ref()
                .is_some_and(|usage| !usage.anomalies.any())
    });
    let mut record = ObservationRecord::new(request_bytes, pending.request, pending.started_at)
        .with_correlation_status(correlation);
    if let Some(streaming) = streaming {
        record = record.with_streaming(streaming);
    }
    if let Some(status_code) = status_code {
        record = record.with_status_code(status_code);
    }
    if let Some(response) = response {
        let outcome = observation_outcome(response.response_state);
        record = record.with_response(response).with_outcome(outcome);
    }
    // The failure class is content-free evidence that this forward reached no response at all,
    // so it overrides any semantic outcome and closes the attempt.
    if let Some(failure) = transport_failure {
        record = record.with_transport_error(failure.label());
    }
    if terminal {
        record = record.with_ended_at(ended_at);
    }
    Some((
        record,
        context,
        shadow_body,
        analysis_permit,
        provider_input_tokens,
        provider_usage_comparable,
    ))
}

pub(super) fn context_block_summaries(
    blocks: &[tracepress_context::ContextBlockDraft],
) -> Vec<ContextBlockSummary> {
    blocks
        .iter()
        .map(|block| ContextBlockSummary {
            exact_fingerprint: block.exact_fingerprint,
            semantic_fingerprint: block.semantic_fingerprint,
            estimated_tokens: block
                .token_estimate
                .as_ref()
                .map(|estimate| estimate.tokens),
        })
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "one bounded pass keeps all context metric precedence together"
)]
#[allow(
    clippy::cast_precision_loss,
    reason = "bounded context token totals are intentionally projected to metric ratios"
)]
pub(crate) fn context_metrics(
    blocks: &[tracepress_context::ContextBlockDraft],
    status: ContextAnalysisStatus,
    stable_explicit_prefix_estimate: Option<u64>,
) -> (ContextAnalysisMetrics, Option<u64>) {
    let analysis_complete = matches!(status, ContextAnalysisStatus::Complete);
    let mut all = TokenEstimateAggregate::new();
    let mut by_kind: [TokenEstimateAggregate; 15] =
        std::array::from_fn(|_| TokenEstimateAggregate::new());
    let mut by_role: [TokenEstimateAggregate; 6] =
        std::array::from_fn(|_| TokenEstimateAggregate::new());
    let mut by_origin: [TokenEstimateAggregate; 8] =
        std::array::from_fn(|_| TokenEstimateAggregate::new());
    let mut confidence = None;
    let mut mixed_confidence = false;
    let mut opportunity_signals = Vec::new();
    let mut exact_groups: HashMap<_, (u64, TokenEstimateAggregate)> = HashMap::new();
    let mut schema_groups: HashMap<_, (u64, TokenEstimateAggregate)> = HashMap::new();
    let mut tool_count = 0_u64;
    let mut schema_bytes = 0_u64;
    let mut largest_tool_schema = 0_u64;
    let mut schema_aggregate = TokenEstimateAggregate::new();

    for block in blocks {
        let estimate = block.token_estimate.as_ref();
        all.observe_estimate(estimate);
        by_kind[context_kind_index(block.kind)].observe_estimate(estimate);
        if let Some(role) = block.role {
            by_role[context_role_index(role)].observe_estimate(estimate);
        }
        by_origin[context_origin_index(block.origin)].observe_estimate(estimate);
        if let Some(estimate) = estimate {
            match confidence {
                None => confidence = Some(estimate.confidence),
                Some(previous) if previous != estimate.confidence => mixed_confidence = true,
                Some(_) => {}
            }
        }
        for signal in block.opportunity_signals.iter() {
            if !opportunity_signals.contains(&signal) {
                opportunity_signals.push(signal);
            }
        }
        let exact_entry = exact_groups
            .entry(block.exact_fingerprint)
            .or_insert_with(|| (0, TokenEstimateAggregate::new()));
        exact_entry.0 = exact_entry.0.saturating_add(1);
        exact_entry.1.observe_estimate(estimate);
        if matches!(block.kind, ContextBlockKind::ToolDefinition) {
            tool_count = tool_count.saturating_add(1);
            schema_bytes = schema_bytes.saturating_add(block.raw_bytes);
            largest_tool_schema = largest_tool_schema.max(block.raw_bytes);
            schema_aggregate.observe_estimate(estimate);
            let schema_entry = schema_groups
                .entry(block.exact_fingerprint)
                .or_insert_with(|| (0, TokenEstimateAggregate::new()));
            schema_entry.0 = schema_entry.0.saturating_add(1);
            schema_entry.1.observe_estimate(estimate);
        }
    }

    let aggregate_total = |aggregate: &TokenEstimateAggregate| {
        if aggregate.observed_blocks() == 0 && !analysis_complete {
            None
        } else {
            aggregate.complete_total()
        }
    };
    let visible_estimated_tokens = aggregate_total(&all);
    let kind_totals = std::array::from_fn(|index| aggregate_total(&by_kind[index]));
    let role_totals = std::array::from_fn(|index| aggregate_total(&by_role[index]));
    let origin_totals = std::array::from_fn(|index| aggregate_total(&by_origin[index]));

    let mut unique_total = Some(0_u64);
    let mut repeated_total = Some(0_u64);
    let mut saw_unique = false;
    let mut saw_repeated = false;
    for (count, aggregate) in exact_groups.values() {
        let total = aggregate.complete_total();
        if *count == 1 {
            saw_unique = true;
            let Some(value) = total else {
                unique_total = None;
                continue;
            };
            if let Some(sum) = unique_total.as_mut() {
                *sum = sum.saturating_add(value);
            }
        } else {
            saw_repeated = true;
            let Some(value) = total else {
                repeated_total = None;
                continue;
            };
            if let Some(sum) = repeated_total.as_mut() {
                *sum = sum.saturating_add(value);
            }
        }
    }
    let share = |part: Option<u64>| {
        visible_estimated_tokens.and_then(|total| {
            if total == 0 {
                None
            } else {
                part.map(|value| value as f64 / total as f64)
            }
        })
    };

    let mut repeated_schema_tokens = Some(0_u64);
    let mut saw_repeated_schema = false;
    for (count, aggregate) in schema_groups.values() {
        if *count <= 1 {
            continue;
        }
        saw_repeated_schema = true;
        let Some(value) = aggregate.complete_total() else {
            repeated_schema_tokens = None;
            continue;
        };
        if let Some(sum) = repeated_schema_tokens.as_mut() {
            *sum = sum.saturating_add(value);
        }
    }
    let estimated_schema_tokens = if tool_count == 0 && analysis_complete {
        Some(0)
    } else {
        aggregate_total(&schema_aggregate)
    };
    let tool_count = if tool_count == 0 && !analysis_complete {
        None
    } else {
        Some(tool_count)
    };
    let schema_bytes = if schema_bytes == 0 && !analysis_complete {
        None
    } else {
        Some(schema_bytes)
    };
    let largest_tool_schema = if largest_tool_schema == 0 && !analysis_complete {
        None
    } else {
        Some(largest_tool_schema)
    };
    let repeated_schema_tokens = if saw_repeated_schema {
        repeated_schema_tokens
    } else if analysis_complete {
        Some(0)
    } else {
        None
    };
    let opportunity_signals = if opportunity_signals.is_empty() && !analysis_complete {
        None
    } else {
        Some(opportunity_signals)
    };

    let mut metrics = ContextAnalysisMetrics::new();
    metrics.explicit_bytes = Some(blocks.iter().map(|block| block.raw_bytes).sum());
    metrics.estimated_tokens_by_kind = kind_totals;
    metrics.estimated_tokens_by_role = role_totals;
    metrics.estimated_tokens_by_origin = origin_totals;
    metrics.estimated_tool_definition_share = share(aggregate_total(
        &by_kind[context_kind_index(ContextBlockKind::ToolDefinition)],
    ));
    metrics.estimated_tool_result_share = share(aggregate_total(
        &by_kind[context_kind_index(ContextBlockKind::ToolResult)],
    ));
    metrics.estimated_human_text_share = share(aggregate_total(
        &by_origin[context_origin_index(ContextOrigin::HumanAuthored)],
    ));
    metrics.estimated_assistant_history_share = share(aggregate_total(
        &by_kind[context_kind_index(ContextBlockKind::AssistantHistory)],
    ));
    metrics.estimated_unique_content_share = if saw_unique {
        share(unique_total)
    } else if analysis_complete {
        Some(0.0)
    } else {
        None
    };
    metrics.estimated_repeated_content_share = if saw_repeated {
        share(repeated_total)
    } else if analysis_complete {
        Some(0.0)
    } else {
        None
    };
    metrics.tool_count = tool_count;
    metrics.schema_bytes = schema_bytes;
    metrics.estimated_schema_tokens = estimated_schema_tokens;
    metrics.largest_tool_schema = largest_tool_schema;
    metrics.repeated_schema_tokens = repeated_schema_tokens;
    metrics.stable_explicit_prefix_estimate = stable_explicit_prefix_estimate;
    metrics.estimator = all.estimator().map(|value| value.as_str().to_owned());
    metrics.estimator_version = all.estimator_version();
    metrics.estimate_confidence = if mixed_confidence { None } else { confidence };
    metrics.opportunity_signals = opportunity_signals;
    (metrics, visible_estimated_tokens)
}

pub(super) const fn context_kind_index(kind: ContextBlockKind) -> usize {
    match kind {
        ContextBlockKind::Instructions => 0,
        ContextBlockKind::Message => 1,
        ContextBlockKind::Text => 2,
        ContextBlockKind::ImageReference => 3,
        ContextBlockKind::FileReference => 4,
        ContextBlockKind::ToolDefinition => 5,
        ContextBlockKind::ToolCall => 6,
        ContextBlockKind::ToolResult => 7,
        ContextBlockKind::ItemReference => 8,
        ContextBlockKind::PromptReference => 9,
        ContextBlockKind::ProviderStateReference => 10,
        ContextBlockKind::AssistantHistory => 11,
        ContextBlockKind::OpaqueReasoning => 12,
        ContextBlockKind::Opaque => 13,
        _ => 14,
    }
}

pub(super) const fn context_role_index(role: ContextRole) -> usize {
    match role {
        ContextRole::System => 0,
        ContextRole::Developer => 1,
        ContextRole::User => 2,
        ContextRole::Assistant => 3,
        ContextRole::Tool => 4,
        _ => 5,
    }
}

pub(super) const fn context_origin_index(origin: ContextOrigin) -> usize {
    match origin {
        ContextOrigin::HumanAuthored => 0,
        ContextOrigin::AgentGenerated => 1,
        ContextOrigin::ToolGenerated => 2,
        ContextOrigin::ToolSchema => 3,
        ContextOrigin::ProviderManaged => 4,
        ContextOrigin::ExternalReference => 5,
        ContextOrigin::TracepressGenerated => 6,
        _ => 7,
    }
}
pub(super) const fn observation_outcome(
    state: ProviderResponseState,
) -> ProviderObservationOutcome {
    match state {
        ProviderResponseState::Completed => ProviderObservationOutcome::Completed,
        ProviderResponseState::Incomplete => ProviderObservationOutcome::Incomplete,
        ProviderResponseState::Failed => ProviderObservationOutcome::Failed,
        ProviderResponseState::Cancelled => ProviderObservationOutcome::Cancelled,
        ProviderResponseState::Disconnected => ProviderObservationOutcome::Disconnected,
        _ => ProviderObservationOutcome::InProgress,
    }
}
