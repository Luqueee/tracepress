//! Shadow and active compression accounting and evaluation.

use super::*;

/// The bounded predecessor retained by the context worker for the next context delta.
#[derive(Debug)]
pub(crate) struct PreviousContextSnapshot {
    pub(super) id: ContextSnapshotId,
    pub(super) blocks: Vec<ContextBlockSummary>,
    pub(super) status: ContextAnalysisStatus,
}

pub(super) const SHADOW_QUEUE_ITEMS: usize = 8;
pub(super) const SHADOW_QUEUE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct ShadowByteBudget {
    used: AtomicU64,
    maximum: u64,
}

impl ShadowByteBudget {
    pub(super) const fn new(maximum: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            maximum,
        }
    }

    pub(super) fn try_acquire(self: &Arc<Self>, bytes: u64) -> Option<ShadowBytePermit> {
        let mut observed = self.used.load(Ordering::Relaxed);
        loop {
            let next = observed.checked_add(bytes)?;
            if next > self.maximum {
                return None;
            }
            match self.used.compare_exchange_weak(
                observed,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(ShadowBytePermit {
                        budget: Arc::clone(self),
                        bytes,
                    });
                }
                Err(actual) => observed = actual,
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct ShadowBytePermit {
    budget: Arc<ShadowByteBudget>,
    bytes: u64,
}

impl Drop for ShadowBytePermit {
    fn drop(&mut self) {
        let previous = self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(previous >= self.bytes, "shadow byte accounting underflow");
    }
}

#[derive(Debug, Default)]
pub(super) struct ShadowCounters {
    drops: AtomicU64,
    jobs_admitted: AtomicU64,
    jobs_processed: AtomicU64,
    job_drops: AtomicU64,
    candidate_evaluations_attempted: AtomicU64,
    candidate_evaluations_completed: AtomicU64,
    candidate_evaluation_drops: AtomicU64,
    queue_full_drops: AtomicU64,
    byte_budget_drops: AtomicU64,
    work_budget_drops: AtomicU64,
    worker_closed_drops: AtomicU64,
    persistence_drops: AtomicU64,
    recovery_failures: AtomicU64,
    determinism_failures: AtomicU64,
}

#[derive(Debug, Default)]
pub(crate) struct ActiveCompressionCounters {
    attempts: AtomicU64,
    rewrites: AtomicU64,
    evaluated_spans: AtomicU64,
    evaluated_input_bytes: AtomicU64,
    evaluated_candidate_bytes: AtomicU64,
    input_bytes: AtomicU64,
    output_bytes: AtomicU64,
    no_improvement: AtomicU64,
    not_applicable: AtomicU64,
    resource_limits: AtomicU64,
    invalid_inputs: AtomicU64,
    recovery_failures: AtomicU64,
    determinism_failures: AtomicU64,
    internal_errors: AtomicU64,
}

impl ActiveCompressionCounters {
    pub(super) fn record(&self, observation: &ActiveCompressionObservation) {
        let _attempt = self.attempts.fetch_add(1, Ordering::Relaxed);
        let metrics = &observation.metrics;
        let _spans = self
            .evaluated_spans
            .fetch_add(u64::from(metrics.evaluated_spans), Ordering::Relaxed);
        let _input = self
            .evaluated_input_bytes
            .fetch_add(metrics.evaluated_input_bytes, Ordering::Relaxed);
        let _candidate = self
            .evaluated_candidate_bytes
            .fetch_add(metrics.evaluated_candidate_bytes, Ordering::Relaxed);
        if matches!(
            metrics.status,
            tracepress_compression::ActiveRewriteStatus::InternalError
                | tracepress_compression::ActiveRewriteStatus::RecoveryFailed
        ) && !metrics.deterministic
        {
            let _count = self.determinism_failures.fetch_add(1, Ordering::Relaxed);
        }
        match metrics.status {
            tracepress_compression::ActiveRewriteStatus::Rewritten => {
                let _rewrite = self.rewrites.fetch_add(1, Ordering::Relaxed);
                let _input = self
                    .input_bytes
                    .fetch_add(metrics.input_bytes, Ordering::Relaxed);
                let _output = self
                    .output_bytes
                    .fetch_add(metrics.output_bytes.unwrap_or_default(), Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::NoImprovement => {
                let _count = self.no_improvement.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::NotApplicable => {
                let _count = self.not_applicable.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::ResourceLimit => {
                let _count = self.resource_limits.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::InvalidInput => {
                let _count = self.invalid_inputs.fetch_add(1, Ordering::Relaxed);
            }
            tracepress_compression::ActiveRewriteStatus::RecoveryFailed => {
                let _count = self.recovery_failures.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                let _count = self.internal_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn report(&self) -> String {
        let input = self.input_bytes.load(Ordering::Relaxed);
        let output = self.output_bytes.load(Ordering::Relaxed);
        let reduction = input.saturating_sub(output);
        format!(
            "active_compression_attempts={} active_compression_rewrites={} active_compression_evaluated_spans={} active_compression_evaluated_input_bytes={} active_compression_evaluated_candidate_bytes={} active_compression_input_bytes={} active_compression_output_bytes={} active_compression_reduction_bytes={} active_compression_no_improvement={} active_compression_not_applicable={} active_compression_resource_limits={} active_compression_invalid_inputs={} active_compression_recovery_failures={} active_compression_determinism_failures={} active_compression_internal_errors={}",
            self.attempts.load(Ordering::Relaxed),
            self.rewrites.load(Ordering::Relaxed),
            self.evaluated_spans.load(Ordering::Relaxed),
            self.evaluated_input_bytes.load(Ordering::Relaxed),
            self.evaluated_candidate_bytes.load(Ordering::Relaxed),
            input,
            output,
            reduction,
            self.no_improvement.load(Ordering::Relaxed),
            self.not_applicable.load(Ordering::Relaxed),
            self.resource_limits.load(Ordering::Relaxed),
            self.invalid_inputs.load(Ordering::Relaxed),
            self.recovery_failures.load(Ordering::Relaxed),
            self.determinism_failures.load(Ordering::Relaxed),
            self.internal_errors.load(Ordering::Relaxed),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ShadowDropReason {
    QueueFull,
    ByteBudget,
    WorkBudget,
    WorkerClosed,
    Persistence,
}

impl ShadowCounters {
    pub(super) fn record_job_admitted(&self) {
        let _count = self.jobs_admitted.fetch_add(1, Ordering::Relaxed);
    }

    fn record_job_processed(&self) {
        let _count = self.jobs_processed.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_job_drop(&self) {
        let _count = self.job_drops.fetch_add(1, Ordering::Relaxed);
    }

    fn record_candidate_attempt(&self) {
        let _count = self
            .candidate_evaluations_attempted
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_candidate_completed(&self) {
        let _count = self
            .candidate_evaluations_completed
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_candidate_drop(&self) {
        let _count = self
            .candidate_evaluation_drops
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_drop(&self, reason: ShadowDropReason) {
        let _total = self.drops.fetch_add(1, Ordering::Relaxed);
        let counter = match reason {
            ShadowDropReason::QueueFull => &self.queue_full_drops,
            ShadowDropReason::ByteBudget => &self.byte_budget_drops,
            ShadowDropReason::WorkBudget => &self.work_budget_drops,
            ShadowDropReason::WorkerClosed => &self.worker_closed_drops,
            ShadowDropReason::Persistence => &self.persistence_drops,
        };
        let _reason_count = counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(super) struct ShadowJob {
    pub(super) snapshot_id: ContextSnapshotId,
    pub(super) analysis: ContextAnalysisResult,
    pub(super) accepted_block_count: u64,
    pub(super) body: ShadowAnalysisBody,
    pub(super) _byte_permit: ShadowBytePermit,
}

pub(super) struct ShadowCompressionWorker {
    pub(super) config: Config,
    pub(super) experiment_id: String,
    pub(super) limits: CompressionLimits,
    pub(super) context_limits: ContextAnalysisLimits,
    pub(super) counters: Arc<ShadowCounters>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ShadowShapeMetadata {
    provider_readability: String,
    json_root_kind: Option<String>,
    json_array_length_bucket: Option<String>,
    json_object_key_count_bucket: Option<String>,
    json_homogeneity_basis_points: Option<u16>,
    json_primitive_cell_ratio_basis_points: Option<u16>,
    json_nested_cell_ratio_basis_points: Option<u16>,
    text_shape: Option<String>,
}

impl ShadowCompressionWorker {
    pub(super) async fn run(
        self,
        mut jobs: tokio::sync::mpsc::Receiver<ShadowJob>,
    ) -> Result<(), String> {
        self.record_manifest("running", None).await?;
        while let Some(job) = jobs.recv().await {
            self.record_job(job).await;
        }
        self.flush_counters().await;
        self.record_manifest("completed", current_timestamp().ok())
            .await
    }

    async fn record_manifest(
        &self,
        status: &str,
        completed_at: Option<String>,
    ) -> Result<(), String> {
        let compressor_set_json = serde_json::to_string(&[
            ("json.noop", 1_u32),
            ("json.minify", 1),
            ("json.tabular", 1),
            ("json.repeated_subtree", 1),
            ("json.readable_table", 1),
            ("json.compact_records", 1),
            ("json.key_elision", 1),
            ("json.empty_noise_fields", 1),
            ("json.repeated_value_elision", 1),
            ("shell.diagnostic_projection", 1),
            ("search.result_projection", 1),
            ("text.noop", 1),
            ("text.repeated_line", 1),
            ("text.repeated_run", 1),
            ("text.readable_line_fold", 1),
            ("text.readable_block_fold", 1),
            ("text.log_prefix_fold", 1),
        ])
        .map_err(|error| error.to_string())?;
        let limits_json = serde_json::to_string(&self.limits).map_err(|error| error.to_string())?;
        let response = control(
            &self.config,
            ControlRequest::RecordShadowExperiment {
                manifest: ShadowExperimentManifest::new(
                    self.experiment_id.clone(),
                    compressor_set_json,
                    option_env!("TRACEPRESS_BUILD_SHA").map(str::to_owned),
                    limits_json,
                    status.to_owned(),
                    current_timestamp()?,
                    completed_at,
                ),
            },
        )
        .await?;
        match response {
            ControlResponse::Ok { .. } => Ok(()),
            _ => Err("daemon rejected shadow experiment manifest".to_owned()),
        }
    }

    async fn record_job(&self, job: ShadowJob) {
        self.counters.record_job_processed();
        let candidates = evaluate_shadow_job(
            &job,
            &self.experiment_id,
            self.limits,
            self.context_limits,
            &self.counters,
        );
        if candidates.is_empty() {
            return;
        }
        self.persist_shadow_candidates(candidates).await;
    }

    async fn persist_shadow_candidates(&self, candidates: Vec<ShadowCandidateRecord>) {
        let mut batch = Vec::new();
        for candidate in candidates {
            let mut proposed = batch.clone();
            proposed.push(candidate.clone());
            if shadow_candidate_batch_fits(&proposed) {
                batch.push(candidate);
                continue;
            }

            if !batch.is_empty() {
                self.persist_shadow_candidate_batch(std::mem::take(&mut batch))
                    .await;
            }
            if shadow_candidate_batch_fits(std::slice::from_ref(&candidate)) {
                batch.push(candidate);
            } else {
                self.report_shadow_persistence_failure(1, "candidate exceeds IPC body limit");
            }
        }
        if !batch.is_empty() {
            self.persist_shadow_candidate_batch(batch).await;
        }
    }

    async fn persist_shadow_candidate_batch(&self, candidates: Vec<ShadowCandidateRecord>) {
        let batch_size = candidates.len();
        let result = control(
            &self.config,
            ControlRequest::RecordShadowCandidates { candidates },
        )
        .await;
        match result {
            Ok(ControlResponse::Ok { .. }) => {}
            Ok(ControlResponse::Error { message }) => {
                self.report_shadow_persistence_failure(batch_size, &message);
            }
            Ok(ControlResponse::Context { .. } | ControlResponse::ContextStatus { .. }) => {
                self.report_shadow_persistence_failure(batch_size, "unexpected daemon response");
            }
            Err(error) => self.report_shadow_persistence_failure(batch_size, &error),
        }
    }

    fn report_shadow_persistence_failure(&self, batch_size: usize, error: &str) {
        // Keep persistence diagnostics metadata-only. Candidate payloads are never logged because
        // shadow inputs may originate from a private workspace.
        eprintln!("shadow candidate persistence failed: batch_size={batch_size} error={error}");
        self.counters.record_drop(ShadowDropReason::Persistence);
    }

    async fn flush_counters(&self) {
        let _ = control(
            &self.config,
            ControlRequest::RecordShadowCounters {
                counters: ShadowExperimentCounters::new(
                    self.experiment_id.clone(),
                    0,
                    self.counters.drops.swap(0, Ordering::AcqRel),
                    self.counters.jobs_admitted.swap(0, Ordering::AcqRel),
                    self.counters.jobs_processed.swap(0, Ordering::AcqRel),
                    self.counters.job_drops.swap(0, Ordering::AcqRel),
                    self.counters
                        .candidate_evaluations_attempted
                        .swap(0, Ordering::AcqRel),
                    self.counters
                        .candidate_evaluations_completed
                        .swap(0, Ordering::AcqRel),
                    self.counters
                        .candidate_evaluation_drops
                        .swap(0, Ordering::AcqRel),
                    self.counters.queue_full_drops.swap(0, Ordering::AcqRel),
                    self.counters.byte_budget_drops.swap(0, Ordering::AcqRel),
                    self.counters.work_budget_drops.swap(0, Ordering::AcqRel),
                    self.counters.worker_closed_drops.swap(0, Ordering::AcqRel),
                    self.counters.persistence_drops.swap(0, Ordering::AcqRel),
                    self.counters.recovery_failures.swap(0, Ordering::AcqRel),
                    self.counters.determinism_failures.swap(0, Ordering::AcqRel),
                ),
            },
        )
        .await;
    }
}

/// Returns whether a shadow-candidate control request stays below the conservative IPC body
/// budget. `IpcRequest` embeds the body as a JSON byte array, so the quarter-frame bound leaves
/// room for that expansion and the authenticated envelope.
pub(crate) fn shadow_candidate_batch_fits(candidates: &[ShadowCandidateRecord]) -> bool {
    let body_limit = usize::try_from(BODY_BYTES / 4).unwrap_or(0);
    let request = ControlRequest::RecordShadowCandidates {
        candidates: candidates.to_vec(),
    };
    serde_json::to_vec(&request).is_ok_and(|body| body.len() <= body_limit)
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "shadow inputs keep independent bounds and accounting explicit"
)]
pub(super) fn evaluate_shadow_job(
    job: &ShadowJob,
    experiment_id: &str,
    limits: CompressionLimits,
    context_limits: ContextAnalysisLimits,
    counters: &ShadowCounters,
) -> Vec<ShadowCandidateRecord> {
    let accepted = usize::try_from(job.accepted_block_count)
        .unwrap_or(job.analysis.blocks.len())
        .min(job.analysis.blocks.len());
    let generator = UuidV7Generator::new();
    let estimator = StructuralHeuristicEstimator::new();
    let json_compressors: [&dyn ShadowCompressor; 7] = [
        &JsonNoop,
        &JsonMinify,
        &JsonTabular,
        &JsonRepeatedSubtree,
        &JsonReadableTable,
        &JsonCompactRecords,
        &JsonKeyElision,
    ];
    let text_compressors: [&dyn ShadowCompressor; 6] = [
        &TextNoop,
        &TextRepeatedLine,
        &TextRepeatedRun,
        &TextReadableLineFold,
        &TextReadableBlockFold,
        &TextLogPrefixFold,
    ];
    let mut records = Vec::new();
    let mut remaining_work = limits.max_shadow_work_units;
    for block in &job.analysis.blocks[..accepted] {
        let metadata = shadow_block_metadata(block, job.body.len());
        let compressors: &[&dyn ShadowCompressor] = match metadata.detected_kind {
            CompressionDetectedKind::Json if metadata.is_tool_result_json() => &json_compressors,
            CompressionDetectedKind::PlainText if metadata.is_tool_result_plain_text() => {
                &text_compressors
            }
            _ => continue,
        };
        let Some(content) = shadow_block_content(job.body.as_ref(), block) else {
            continue;
        };
        let shape = classify_shadow_shape(&content, metadata.detected_kind);
        let shell_generic = block
            .tool_name
            .as_ref()
            .map(|name| ToolFamily::from_tool_name(Some(name.as_str())))
            == Some(ToolFamily::ShellGeneric);
        // Search eligibility is proved by the transient ToolCall/ToolResult pair, not by the
        // provider's tool-name label alone. Native providers may call the shell surface by a
        // different name while still carrying an unambiguous bounded `rg` command. The command
        // and output are discarded after this in-memory classification.
        let shell_family =
            classify_shell_semantic_family(&job.analysis.blocks, block, job.body.as_ref());
        let reduction_reducers: [(&dyn ToolResultReducer, bool); 4] = [
            (&JsonEmptyNoiseFieldReducer, true),
            (&JsonRepeatedValueReducer, true),
            (&ShellDiagnosticProjectionReducer, shell_generic),
            (
                &SearchResultReducer,
                // Provider-native shell labels and call-id correlation can be absent even when
                // the ToolResult is a JSON envelope. The reducer itself requires one canonical
                // Search payload, so evaluating JSON ToolResults in shadow is still fail-closed
                // and gives an explicit `not_applicable` record instead of silent missingness.
                shell_family == ShellSemanticFamily::Search || metadata.is_tool_result_json(),
            ),
        ];
        let max_candidates = usize::try_from(limits.max_candidates_per_block).unwrap_or(usize::MAX);
        for (candidate_index, compressor) in compressors.iter().enumerate() {
            if candidate_index >= max_candidates {
                counters.record_candidate_drop();
                continue;
            }
            let work = u64::try_from(content.len()).unwrap_or(u64::MAX).max(1);
            let Some(remaining) = remaining_work.checked_sub(work) else {
                counters.record_candidate_drop();
                counters.record_drop(ShadowDropReason::WorkBudget);
                continue;
            };
            remaining_work = remaining;
            counters.record_candidate_attempt();
            let candidate =
                evaluate_with_estimator(*compressor, metadata, &content, &limits, |bytes| {
                    estimator
                        .estimate(&EstimationRequest {
                            model: None,
                            content: bytes,
                            limits: context_limits,
                        })
                        .tokens()
                });
            let metrics = candidate.metrics();
            counters.record_candidate_completed();
            if matches!(metrics.status, CandidateStatus::RecoveryFailed) {
                let _counted = counters.recovery_failures.fetch_add(1, Ordering::Relaxed);
            }
            if !metrics.deterministic && metrics.candidate_fingerprint.is_some() {
                let _counted = counters
                    .determinism_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
            records.push(shadow_candidate_record(
                &generator,
                experiment_id,
                job.snapshot_id,
                u64::from(block.ordinal),
                metrics,
                &shape,
            ));
        }
        for (reduction_index, (reduction_reducer, enabled)) in reduction_reducers.iter().enumerate()
        {
            if !enabled {
                continue;
            }
            let candidate_index = json_compressors.len().saturating_add(reduction_index);
            if candidate_index >= max_candidates {
                counters.record_candidate_drop();
                continue;
            }
            if !matches!(
                reduction_reducer
                    .policy(metadata, u64::try_from(content.len()).unwrap_or(u64::MAX)),
                ReductionPolicyDecision::Reduce | ReductionPolicyDecision::KeepFullWithCandidate
            ) {
                continue;
            }
            // A reducer is evaluated twice for determinism. Reserve a conservative shared
            // budget for both passes so the shadow worker can never outrun its job bound.
            let work = u64::try_from(content.len())
                .unwrap_or(u64::MAX)
                .max(1)
                .saturating_mul(2);
            let Some(remaining) = remaining_work.checked_sub(work) else {
                counters.record_candidate_drop();
                counters.record_drop(ShadowDropReason::WorkBudget);
                continue;
            };
            remaining_work = remaining;
            counters.record_candidate_attempt();
            let reduced = reduction_reducer.reduce(&content, &limits);
            let repeat = reduction_reducer.reduce(&content, &limits);
            let deterministic = reduced.metrics().visible_fingerprint
                == repeat.metrics().visible_fingerprint
                && reduced.metrics().status == repeat.metrics().status;
            let visible_estimate = reduced.visible().and_then(|bytes| {
                estimator
                    .estimate(&EstimationRequest {
                        model: None,
                        content: bytes,
                        limits: context_limits,
                    })
                    .tokens()
            });
            let reduced = reduced
                .with_estimates(metadata.input_estimated_tokens, visible_estimate)
                .with_deterministic(deterministic);
            counters.record_candidate_completed();
            let metrics = reduced.metrics();
            records.push(shadow_reduction_candidate_record(
                &generator,
                experiment_id,
                job.snapshot_id,
                u64::from(block.ordinal),
                metrics,
                &shape,
            ));
        }
    }
    records
}

pub(super) fn classify_shell_semantic_family(
    blocks: &[tracepress_context::ContextBlockDraft],
    result: &tracepress_context::ContextBlockDraft,
    body: &[u8],
) -> ShellSemanticFamily {
    let command = result
        .tool_call_id
        .as_ref()
        .and_then(|result_call_id| {
            blocks.iter().find(|candidate| {
                candidate.kind == ContextBlockKind::ToolCall
                    && candidate
                        .tool_call_id
                        .as_ref()
                        .is_some_and(|call_id| call_id.as_str() == result_call_id.as_str())
            })
        })
        .and_then(|call| shadow_block_content(body, call));
    let output = shadow_block_content(body, result);
    ShellSemanticFamily::from_transient_signals(command.as_deref(), output.as_deref())
}

pub(super) fn shadow_block_content(
    body: &[u8],
    block: &tracepress_context::ContextBlockDraft,
) -> Option<Vec<u8>> {
    let [raw_start, raw_end] = block
        .content_span
        .unwrap_or([block.locator.raw_value_start, block.locator.raw_value_end]);
    let start = usize::try_from(raw_start).ok()?;
    let end = usize::try_from(raw_end).ok()?;
    let raw = body.get(start..end)?;
    if raw.first() == Some(&b'"') {
        serde_json::from_slice::<String>(raw)
            .ok()
            .map(String::into_bytes)
    } else {
        Some(raw.to_vec())
    }
}

pub(super) fn shadow_block_metadata(
    block: &tracepress_context::ContextBlockDraft,
    request_analysis_bytes: usize,
) -> BlockMetadata {
    let origin = match block.origin {
        ContextOrigin::ToolGenerated => CompressionBlockOrigin::ToolGenerated,
        ContextOrigin::HumanAuthored => CompressionBlockOrigin::HumanAuthored,
        ContextOrigin::AgentGenerated => CompressionBlockOrigin::AgentGenerated,
        ContextOrigin::ToolSchema => CompressionBlockOrigin::ToolSchema,
        ContextOrigin::ProviderManaged => CompressionBlockOrigin::ProviderManaged,
        _ => CompressionBlockOrigin::Other,
    };
    let kind = if block.kind == ContextBlockKind::ToolResult {
        CompressionBlockKind::ToolResult
    } else {
        CompressionBlockKind::Other
    };
    let detected_kind = match block.detection_result.as_ref().map(|value| value.kind) {
        Some(DetectedContentKind::Json) => CompressionDetectedKind::Json,
        Some(DetectedContentKind::PlainText) => CompressionDetectedKind::PlainText,
        Some(DetectedContentKind::Unknown) | None => CompressionDetectedKind::Unknown,
        _ => CompressionDetectedKind::Other,
    };
    BlockMetadata::new(
        origin,
        kind,
        detected_kind,
        block.token_estimate.as_ref().map(|value| value.tokens),
        block.locator.raw_value_start,
        u64::try_from(request_analysis_bytes).unwrap_or(u64::MAX),
        None,
        None,
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "the metadata-only shape classifier keeps all bounded shape rules together"
)]
pub(super) fn classify_shadow_shape(
    input: &[u8],
    detected_kind: CompressionDetectedKind,
) -> ShadowShapeMetadata {
    let provider_readability = "unknown";
    match detected_kind {
        CompressionDetectedKind::Json => {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(input) else {
                return ShadowShapeMetadata {
                    provider_readability: provider_readability.to_owned(),
                    ..ShadowShapeMetadata::default()
                };
            };
            let mut shape = ShadowShapeMetadata {
                provider_readability: provider_readability.to_owned(),
                ..ShadowShapeMetadata::default()
            };
            match &value {
                serde_json::Value::Array(rows) => {
                    shape.json_array_length_bucket = Some(length_bucket(rows.len()));
                    if rows.iter().all(serde_json::Value::is_object) {
                        shape.json_root_kind = Some("array_object".to_owned());
                        let objects: Vec<&serde_json::Map<String, serde_json::Value>> = rows
                            .iter()
                            .filter_map(serde_json::Value::as_object)
                            .collect();
                        let first_keys = objects
                            .first()
                            .map(|object| object.keys().cloned().collect::<BTreeSet<_>>())
                            .unwrap_or_default();
                        let homogeneous = objects
                            .iter()
                            .filter(|object| {
                                object.keys().cloned().collect::<BTreeSet<_>>() == first_keys
                            })
                            .count();
                        shape.json_homogeneity_basis_points =
                            Some(ratio_basis_points(homogeneous, objects.len()));
                        shape.json_object_key_count_bucket = Some(length_bucket(first_keys.len()));
                        let mut primitive = 0_usize;
                        let mut nested = 0_usize;
                        for object in &objects {
                            for cell in object.values() {
                                if cell.is_array() || cell.is_object() {
                                    nested = nested.saturating_add(1);
                                } else {
                                    primitive = primitive.saturating_add(1);
                                }
                            }
                        }
                        let total = primitive.saturating_add(nested);
                        shape.json_primitive_cell_ratio_basis_points =
                            Some(ratio_basis_points(primitive, total));
                        shape.json_nested_cell_ratio_basis_points =
                            Some(ratio_basis_points(nested, total));
                    } else if rows
                        .iter()
                        .all(|value| value.is_object() || value.is_array())
                    {
                        shape.json_root_kind = Some("array_nested".to_owned());
                    } else if rows
                        .iter()
                        .all(|value| !value.is_object() && !value.is_array())
                    {
                        shape.json_root_kind = Some("array_scalar".to_owned());
                    } else {
                        shape.json_root_kind = Some("array_heterogeneous".to_owned());
                    }
                }
                serde_json::Value::Object(object) => {
                    shape.json_root_kind = Some("object".to_owned());
                    shape.json_object_key_count_bucket = Some(length_bucket(object.len()));
                }
                serde_json::Value::String(_) => shape.json_root_kind = Some("string".to_owned()),
                serde_json::Value::Number(_) => shape.json_root_kind = Some("number".to_owned()),
                serde_json::Value::Bool(_) => shape.json_root_kind = Some("boolean".to_owned()),
                serde_json::Value::Null => shape.json_root_kind = Some("null".to_owned()),
            }
            shape
        }
        CompressionDetectedKind::PlainText => {
            let text = String::from_utf8_lossy(input);
            let lines: Vec<&str> = text.lines().collect();
            let mut counts = BTreeMap::<&str, usize>::new();
            for line in &lines {
                let count = counts.entry(line).or_default();
                *count = count.saturating_add(1);
            }
            let duplicate_lines = counts.values().any(|count| *count > 1);
            let log_like = lines
                .iter()
                .filter(|line| line.split_whitespace().count() >= 3 && line.contains(' '))
                .count();
            ShadowShapeMetadata {
                provider_readability: provider_readability.to_owned(),
                text_shape: Some(
                    if duplicate_lines {
                        "duplicate_lines"
                    } else if log_like.saturating_mul(2) >= lines.len().max(1) {
                        "logs_like"
                    } else {
                        "plain"
                    }
                    .to_owned(),
                ),
                ..ShadowShapeMetadata::default()
            }
        }
        _ => ShadowShapeMetadata {
            provider_readability: provider_readability.to_owned(),
            ..ShadowShapeMetadata::default()
        },
    }
}

pub(super) fn length_bucket(length: usize) -> String {
    match length {
        0 => "0".to_owned(),
        1..=1 => "1".to_owned(),
        2..=9 => "2-9".to_owned(),
        10..=99 => "10-99".to_owned(),
        100..=999 => "100-999".to_owned(),
        _ => "1000+".to_owned(),
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "bounded ratio conversion saturates before narrowing"
)]
pub(super) fn ratio_basis_points(numerator: usize, denominator: usize) -> u16 {
    if denominator == 0 {
        return 0;
    }
    u16::try_from((numerator.saturating_mul(10_000) / denominator).min(10_000)).unwrap_or(10_000)
}

pub(super) fn compressor_provider_readability(compressor_id: &str) -> String {
    match compressor_id {
        "json.readable_table"
        | "json.compact_records"
        | "json.key_elision"
        | "text.readable_line_fold"
        | "text.readable_block_fold"
        | "text.log_prefix_fold" => "human_readable_structured".to_owned(),
        "json.noop" | "json.minify" | "text.noop" => "provider_compatible_control".to_owned(),
        _ => "opaque_custom_encoding".to_owned(),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "candidate association metadata remains explicit"
)]
pub(super) fn shadow_candidate_record(
    generator: &UuidV7Generator,
    experiment_id: &str,
    snapshot_id: ContextSnapshotId,
    block_ordinal: u64,
    metrics: &CandidateMetrics,
    shape: &ShadowShapeMetadata,
) -> ShadowCandidateRecord {
    let mut shape = shape.clone();
    shape.provider_readability = compressor_provider_readability(&metrics.compressor_id);
    ShadowCandidateRecord {
        candidate_id: CompressionCandidateId::generate(generator),
        experiment_id: experiment_id.to_owned(),
        snapshot_id,
        block_ordinal,
        compressor_id: metrics.compressor_id.clone(),
        compressor_version: metrics.compressor_version.to_string(),
        status: storage_shadow_status(metrics.status),
        input_bytes: metrics.input_bytes,
        output_bytes: metrics.output_bytes,
        bytes_delta: metrics
            .bytes_delta
            .and_then(|value| u64::try_from(value).ok()),
        input_estimated_tokens: metrics.input_estimated_tokens,
        output_estimated_tokens: metrics.output_estimated_tokens,
        estimated_token_delta: metrics
            .estimated_token_delta
            .and_then(|value| u64::try_from(value).ok()),
        processing_us: metrics.processing_us,
        reversible: metrics.reversible,
        recovery_verified: metrics.recovery_verified,
        deterministic: metrics.deterministic,
        original_fingerprint: metrics.original_fingerprint.to_vec().into_boxed_slice(),
        candidate_fingerprint: metrics
            .candidate_fingerprint
            .map(|value| value.to_vec().into_boxed_slice()),
        recovered_fingerprint: metrics
            .recovery_verified
            .then(|| metrics.original_fingerprint.to_vec().into_boxed_slice()),
        first_modified_offset: metrics.first_modified_offset,
        preserved_prefix_bytes: metrics.preserved_prefix_bytes,
        preserved_prefix_ratio_basis_points: metrics.preserved_prefix_ratio_basis_points,
        cache_risk: storage_cache_risk(metrics.cache_risk),
        provider_readability: shape.provider_readability,
        json_root_kind: shape.json_root_kind,
        json_array_length_bucket: shape.json_array_length_bucket,
        json_object_key_count_bucket: shape.json_object_key_count_bucket,
        json_homogeneity_basis_points: shape.json_homogeneity_basis_points,
        json_primitive_cell_ratio_basis_points: shape.json_primitive_cell_ratio_basis_points,
        json_nested_cell_ratio_basis_points: shape.json_nested_cell_ratio_basis_points,
        text_shape: shape.text_shape,
        verified_at_us: current_timestamp_us().ok(),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "reduction association metadata remains explicit"
)]
pub(super) fn shadow_reduction_candidate_record(
    generator: &UuidV7Generator,
    experiment_id: &str,
    snapshot_id: ContextSnapshotId,
    block_ordinal: u64,
    metrics: &ReductionMetrics,
    shape: &ShadowShapeMetadata,
) -> ShadowCandidateRecord {
    ShadowCandidateRecord {
        candidate_id: CompressionCandidateId::generate(generator),
        experiment_id: experiment_id.to_owned(),
        snapshot_id,
        block_ordinal,
        compressor_id: metrics.reducer_id.to_owned(),
        compressor_version: metrics.reducer_version.to_string(),
        status: storage_reduction_status(metrics.status),
        input_bytes: metrics.input_bytes,
        output_bytes: metrics.visible_bytes,
        bytes_delta: metrics.gross_bytes_delta,
        input_estimated_tokens: metrics.input_estimated_tokens,
        output_estimated_tokens: metrics.visible_estimated_tokens,
        estimated_token_delta: metrics.gross_estimated_token_delta,
        processing_us: metrics.processing_us,
        reversible: metrics.recovery_available,
        recovery_verified: metrics.recovery_verified,
        deterministic: metrics.deterministic,
        original_fingerprint: metrics.original_fingerprint.to_vec().into_boxed_slice(),
        candidate_fingerprint: metrics
            .visible_fingerprint
            .map(|value| value.to_vec().into_boxed_slice()),
        recovered_fingerprint: metrics
            .recovery_verified
            .then(|| metrics.original_fingerprint.to_vec().into_boxed_slice()),
        first_modified_offset: metrics.first_modified_offset,
        preserved_prefix_bytes: metrics.preserved_prefix_bytes,
        preserved_prefix_ratio_basis_points: None,
        cache_risk: ShadowCacheRisk::Unknown,
        provider_readability: metrics.provider_readability.to_owned(),
        json_root_kind: shape.json_root_kind.clone(),
        json_array_length_bucket: shape.json_array_length_bucket.clone(),
        json_object_key_count_bucket: shape.json_object_key_count_bucket.clone(),
        json_homogeneity_basis_points: shape.json_homogeneity_basis_points,
        json_primitive_cell_ratio_basis_points: shape.json_primitive_cell_ratio_basis_points,
        json_nested_cell_ratio_basis_points: shape.json_nested_cell_ratio_basis_points,
        text_shape: shape.text_shape.clone(),
        verified_at_us: current_timestamp_us().ok(),
    }
}

pub(super) const fn storage_shadow_status(status: CandidateStatus) -> ShadowCandidateStatus {
    match status {
        CandidateStatus::Applicable => ShadowCandidateStatus::Applicable,
        CandidateStatus::NotApplicable => ShadowCandidateStatus::NotApplicable,
        CandidateStatus::NoImprovement => ShadowCandidateStatus::NoImprovement,
        CandidateStatus::ResourceLimit => ShadowCandidateStatus::ResourceLimit,
        CandidateStatus::InvalidInput => ShadowCandidateStatus::InvalidInput,
        CandidateStatus::RecoveryFailed => ShadowCandidateStatus::RecoveryFailed,
        _ => ShadowCandidateStatus::InternalError,
    }
}

pub(super) const fn storage_reduction_status(status: ReductionStatus) -> ShadowCandidateStatus {
    match status {
        ReductionStatus::Applicable => ShadowCandidateStatus::Applicable,
        ReductionStatus::NotApplicable => ShadowCandidateStatus::NotApplicable,
        ReductionStatus::NoImprovement => ShadowCandidateStatus::NoImprovement,
        ReductionStatus::ResourceLimit => ShadowCandidateStatus::ResourceLimit,
        ReductionStatus::InvalidInput => ShadowCandidateStatus::InvalidInput,
        _ => ShadowCandidateStatus::InternalError,
    }
}

pub(super) const fn storage_cache_risk(risk: CompressionCacheRisk) -> ShadowCacheRisk {
    match risk {
        CompressionCacheRisk::Low => ShadowCacheRisk::Low,
        CompressionCacheRisk::Medium => ShadowCacheRisk::Medium,
        CompressionCacheRisk::High => ShadowCacheRisk::High,
        _ => ShadowCacheRisk::Unknown,
    }
}
