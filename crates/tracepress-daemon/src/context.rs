#![allow(
    missing_docs,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the compact context wire and storage mapping are one versioned boundary"
)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracepress_context::{
    BoundedMetadataText, ContextAnalysisDropReason, ContextAnalysisLimits,
    ContextAnalysisStatus as AnalyzerAnalysisStatus, ContextBlockDraft,
    ContextBlockKind as AnalyzerBlockKind, ContextDelta, ContextDigest,
    ContextOrigin as AnalyzerOrigin, ContextRole as AnalyzerRole, ContextVisibility,
    DetectedContentKind as AnalyzerDetectedKind, DetectionConfidence,
    EstimateConfidence as AnalyzerEstimateConfidence, FeatureRatio, LogicalContextStatus,
    OpportunitySignal as AnalyzerOpportunitySignal,
    ReconciliationStatus as AnalyzerReconciliationStatus, TokenReconciliation,
};
use tracepress_core::{
    AttemptId, ContextBlockOccurrenceId, ContextSnapshotId, OperationId, RequestId, SessionId,
    UuidV7Generator,
};
use tracepress_storage::{
    ContextBlockKind, ContextCorrelationStatus, ContextOrigin, ContextRole, DetectedContentKind,
    EstimateConfidence, EstimatedTokenComposition, EstimatedTokensByKind, EstimatedTokensByOrigin,
    EstimatedTokensByRole, OpportunitySignal, ReconciliationStatus, WriteBatch, WriteCommand,
};

use crate::{
    ActiveContextAnalysis, DaemonError, DaemonService, context_analysis_abort_batch,
    map_analysis_status,
};
use tracepress_core::SessionState;

const MAX_ACTIVE_CONTEXT_ANALYSES: usize = 64;

/// Compact metrics carried by the bounded daemon IPC protocol.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ContextAnalysisMetrics {
    pub explicit_bytes: Option<u64>,
    pub unknown_block_count: Option<u64>,
    pub semantic_coverage_basis_points: Option<u16>,
    pub estimated_tokens_by_kind: [Option<u64>; 15],
    pub estimated_tokens_by_role: [Option<u64>; 6],
    pub estimated_tokens_by_origin: [Option<u64>; 8],
    pub estimated_tool_definition_share: Option<f64>,
    pub estimated_tool_result_share: Option<f64>,
    pub estimated_human_text_share: Option<f64>,
    pub estimated_assistant_history_share: Option<f64>,
    pub estimated_unique_content_share: Option<f64>,
    pub estimated_repeated_content_share: Option<f64>,
    pub tool_count: Option<u64>,
    pub schema_bytes: Option<u64>,
    pub estimated_schema_tokens: Option<u64>,
    pub largest_tool_schema: Option<u64>,
    pub repeated_schema_tokens: Option<u64>,
    pub stable_explicit_prefix_estimate: Option<u64>,
    pub estimator: Option<String>,
    pub estimator_version: Option<u32>,
    pub estimate_confidence: Option<AnalyzerEstimateConfidence>,
    pub opportunity_signals: Option<Vec<AnalyzerOpportunitySignal>>,
}

impl ContextAnalysisMetrics {
    /// Creates an empty metrics payload whose optional values are all absent.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            explicit_bytes: None,
            unknown_block_count: None,
            semantic_coverage_basis_points: None,
            estimated_tokens_by_kind: [None; 15],
            estimated_tokens_by_role: [None; 6],
            estimated_tokens_by_origin: [None; 8],
            estimated_tool_definition_share: None,
            estimated_tool_result_share: None,
            estimated_human_text_share: None,
            estimated_assistant_history_share: None,
            estimated_unique_content_share: None,
            estimated_repeated_content_share: None,
            tool_count: None,
            schema_bytes: None,
            estimated_schema_tokens: None,
            largest_tool_schema: None,
            repeated_schema_tokens: None,
            stable_explicit_prefix_estimate: None,
            estimator: None,
            estimator_version: None,
            estimate_confidence: None,
            opportunity_signals: None,
        }
    }
}

/// Terminal payload for one context analysis.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ContextAnalysisFinalize {
    pub snapshot_id: ContextSnapshotId,
    pub status: AnalyzerAnalysisStatus,
    pub completed_at_us: u64,
    pub request_content_hash: Option<ContextDigest>,
    pub analysis_content_hash: Option<ContextDigest>,
    pub explicit_block_count: Option<u64>,
    pub analyzed_bytes: Option<u64>,
    pub skipped_bytes: Option<u64>,
    pub visibility: ContextVisibility,
    pub duplicate_key_detected: Option<bool>,
    pub reference_resolved_locally: Option<bool>,
    pub correlation_status: ContextCorrelationStatusWire,
    pub metrics: ContextAnalysisMetrics,
    pub delta: Option<ContextDelta>,
    pub reconciliation: TokenReconciliation,
    pub attempt_id: Option<AttemptId>,
}

/// Inputs required to begin one bounded context analysis.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ContextAnalysisBegin {
    pub session_id: SessionId,
    pub provider_request_id: RequestId,
    pub inference_operation_id: OperationId,
    pub analysis_version: u32,
    pub started_at_us: u64,
}

impl ContextAnalysisBegin {
    /// Creates a begin request from its fixed identity and timing fields.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the compact begin request has five fixed identity and timing fields"
    )]
    pub const fn new(
        session_id: SessionId,
        provider_request_id: RequestId,
        inference_operation_id: OperationId,
        analysis_version: u32,
        started_at_us: u64,
    ) -> Self {
        Self {
            session_id,
            provider_request_id,
            inference_operation_id,
            analysis_version,
            started_at_us,
        }
    }
}

/// Missing required input while constructing a context finalize payload.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ContextAnalysisFinalizeBuildError {
    #[error("context finalize builder is missing `{0}`")]
    MissingField(&'static str),
}

/// Builder for the terminal context analysis payload.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ContextAnalysisFinalizeBuilder {
    snapshot_id: ContextSnapshotId,
    status: AnalyzerAnalysisStatus,
    completed_at_us: u64,
    request_content_hash: Option<ContextDigest>,
    analysis_content_hash: Option<ContextDigest>,
    explicit_block_count: Option<u64>,
    analyzed_bytes: Option<u64>,
    skipped_bytes: Option<u64>,
    visibility: Option<ContextVisibility>,
    duplicate_key_detected: Option<bool>,
    reference_resolved_locally: Option<bool>,
    correlation_status: Option<ContextCorrelationStatusWire>,
    metrics: Option<ContextAnalysisMetrics>,
    delta: Option<ContextDelta>,
    reconciliation: Option<TokenReconciliation>,
    attempt_id: Option<AttemptId>,
}

impl ContextAnalysisFinalize {
    /// Starts building a terminal payload from its identity and status.
    #[must_use]
    pub const fn builder(
        snapshot_id: ContextSnapshotId,
        status: AnalyzerAnalysisStatus,
        completed_at_us: u64,
    ) -> ContextAnalysisFinalizeBuilder {
        ContextAnalysisFinalizeBuilder::new(snapshot_id, status, completed_at_us)
    }
}

impl ContextAnalysisFinalizeBuilder {
    /// Starts building a terminal payload from its identity and status.
    #[must_use]
    pub const fn new(
        snapshot_id: ContextSnapshotId,
        status: AnalyzerAnalysisStatus,
        completed_at_us: u64,
    ) -> Self {
        Self {
            snapshot_id,
            status,
            completed_at_us,
            request_content_hash: None,
            analysis_content_hash: None,
            explicit_block_count: None,
            analyzed_bytes: None,
            skipped_bytes: None,
            visibility: None,
            duplicate_key_detected: None,
            reference_resolved_locally: None,
            correlation_status: None,
            metrics: None,
            delta: None,
            reconciliation: None,
            attempt_id: None,
        }
    }

    /// Sets the observed request-content digest.
    #[must_use]
    pub const fn request_content_hash(mut self, value: Option<ContextDigest>) -> Self {
        self.request_content_hash = value;
        self
    }

    /// Sets the digest of the bytes presented to the analyzer.
    #[must_use]
    pub const fn analysis_content_hash(mut self, value: Option<ContextDigest>) -> Self {
        self.analysis_content_hash = value;
        self
    }

    /// Sets the number of explicit blocks accepted for the snapshot.
    #[must_use]
    pub const fn explicit_block_count(mut self, value: Option<u64>) -> Self {
        self.explicit_block_count = value;
        self
    }

    /// Sets the number of analyzed bytes.
    #[must_use]
    pub const fn analyzed_bytes(mut self, value: Option<u64>) -> Self {
        self.analyzed_bytes = value;
        self
    }

    /// Sets the number of skipped bytes.
    #[must_use]
    pub const fn skipped_bytes(mut self, value: Option<u64>) -> Self {
        self.skipped_bytes = value;
        self
    }

    /// Sets the visibility summary.
    #[must_use]
    pub const fn visibility(mut self, value: ContextVisibility) -> Self {
        self.visibility = Some(value);
        self
    }

    /// Sets whether duplicate keys were observed.
    #[must_use]
    pub const fn duplicate_key_detected(mut self, value: Option<bool>) -> Self {
        self.duplicate_key_detected = value;
        self
    }

    /// Sets whether referenced content was resolved locally.
    #[must_use]
    pub const fn reference_resolved_locally(mut self, value: Option<bool>) -> Self {
        self.reference_resolved_locally = value;
        self
    }

    /// Sets the context-to-forward correlation state.
    #[must_use]
    pub const fn correlation_status(mut self, value: ContextCorrelationStatusWire) -> Self {
        self.correlation_status = Some(value);
        self
    }

    /// Sets the aggregate metrics.
    #[must_use]
    pub fn metrics(mut self, value: ContextAnalysisMetrics) -> Self {
        self.metrics = Some(value);
        self
    }

    /// Sets the optional snapshot delta.
    #[must_use]
    pub const fn delta(mut self, value: Option<ContextDelta>) -> Self {
        self.delta = value;
        self
    }

    /// Sets token reconciliation evidence.
    #[must_use]
    pub const fn reconciliation(mut self, value: TokenReconciliation) -> Self {
        self.reconciliation = Some(value);
        self
    }

    /// Sets the provider-attempt identity.
    #[must_use]
    pub const fn attempt_id(mut self, value: Option<AttemptId>) -> Self {
        self.attempt_id = value;
        self
    }

    /// Completes the terminal payload after all required evidence is supplied.
    ///
    /// # Errors
    /// Returns the missing required field when visibility, correlation status, metrics, or
    /// reconciliation evidence was not supplied.
    pub fn build(self) -> Result<ContextAnalysisFinalize, ContextAnalysisFinalizeBuildError> {
        let Some(visibility) = self.visibility else {
            return Err(ContextAnalysisFinalizeBuildError::MissingField(
                "visibility",
            ));
        };
        let Some(correlation_status) = self.correlation_status else {
            return Err(ContextAnalysisFinalizeBuildError::MissingField(
                "correlation_status",
            ));
        };
        let Some(metrics) = self.metrics else {
            return Err(ContextAnalysisFinalizeBuildError::MissingField("metrics"));
        };
        let Some(reconciliation) = self.reconciliation else {
            return Err(ContextAnalysisFinalizeBuildError::MissingField(
                "reconciliation",
            ));
        };
        Ok(ContextAnalysisFinalize {
            snapshot_id: self.snapshot_id,
            status: self.status,
            completed_at_us: self.completed_at_us,
            request_content_hash: self.request_content_hash,
            analysis_content_hash: self.analysis_content_hash,
            explicit_block_count: self.explicit_block_count,
            analyzed_bytes: self.analyzed_bytes,
            skipped_bytes: self.skipped_bytes,
            visibility,
            duplicate_key_detected: self.duplicate_key_detected,
            reference_resolved_locally: self.reference_resolved_locally,
            correlation_status,
            metrics,
            delta: self.delta,
            reconciliation,
            attempt_id: self.attempt_id,
        })
    }
}

/// Explicitly aborts one active context analysis with a typed drop reason.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ContextAnalysisAbort {
    pub snapshot_id: ContextSnapshotId,
    pub reason: ContextAnalysisDropReason,
    pub completed_at_us: u64,
}

impl ContextAnalysisAbort {
    /// Creates an abort request for one active snapshot.
    #[must_use]
    pub const fn new(
        snapshot_id: ContextSnapshotId,
        reason: ContextAnalysisDropReason,
        completed_at_us: u64,
    ) -> Self {
        Self {
            snapshot_id,
            reason,
            completed_at_us,
        }
    }
}

/// Correlation state recorded with a context snapshot.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextCorrelationStatusWire {
    Correlated,
    Degraded,
}

/// One bounded append of analyzer-produced block drafts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ContextBlockBatch {
    pub snapshot_id: ContextSnapshotId,
    pub sequence: u32,
    pub blocks: Vec<ContextBlockDraft>,
}

impl ContextBlockBatch {
    #[must_use]
    pub const fn new(
        snapshot_id: ContextSnapshotId,
        sequence: u32,
        blocks: Vec<ContextBlockDraft>,
    ) -> Self {
        Self {
            snapshot_id,
            sequence,
            blocks,
        }
    }
}
/// Capacity state after one context-block append commits.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextAppendCapacity {
    /// Both block and batch dimensions still accept another append.
    Available,
    /// The block bound was exactly reached; another block is rejected.
    BlocksExhausted,
    /// The batch bound was exactly reached; another batch is rejected.
    BatchesExhausted,
    /// Both block and batch bounds were exactly reached.
    BothExhausted,
}

impl ContextAppendCapacity {
    /// Returns whether another append would exceed a bounded dimension.
    #[must_use]
    pub const fn is_exhausted(self) -> bool {
        !matches!(self, Self::Available)
    }
}

/// Receipt for one durably appended context-block batch.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ContextAppendReceipt {
    /// Cumulative number of blocks accepted for the snapshot.
    pub accepted_block_count: u64,
    /// Sequence expected by the next append.
    pub next_sequence: u32,
    /// Capacity state after the append.
    pub capacity: ContextAppendCapacity,
    /// Current bounded analysis status after the append.
    pub status: AnalyzerAnalysisStatus,
}

impl DaemonService {
    /// Begins a context snapshot after validating its Phase 2 identities.
    ///
    /// # Errors
    /// Returns an association, session-state, context-limit, or storage error when the request
    /// cannot be admitted and durably recorded.
    pub async fn begin_context_analysis(
        &self,
        request: ContextAnalysisBegin,
    ) -> Result<ContextSnapshotId, DaemonError> {
        let ContextAnalysisBegin {
            session_id,
            provider_request_id,
            inference_operation_id,
            analysis_version,
            started_at_us,
        } = request;
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .ok_or(DaemonError::UnknownSession { session_id })?;
        if session.state != SessionState::Active {
            return Err(DaemonError::SessionNotActive {
                session_id,
                state: session.state,
            });
        }
        let operation_is_context_capable = session
            .operations
            .get(&inference_operation_id)
            .is_some_and(|kind| {
                matches!(
                    kind,
                    tracepress_core::OperationKind::LlmInference
                        | tracepress_core::OperationKind::ContextCompaction
                )
            });
        if !operation_is_context_capable
            || session.provider_requests.get(&inference_operation_id) != Some(&provider_request_id)
        {
            return Err(DaemonError::InvalidContextAssociation { session_id });
        }

        let mut analyses = self.context_analyses.lock().await;
        if analyses.len() >= MAX_ACTIVE_CONTEXT_ANALYSES {
            return Err(DaemonError::ContextAnalysisCapacity {
                limit: MAX_ACTIVE_CONTEXT_ANALYSES,
            });
        }
        let ids = self.ids.lock().await;
        let snapshot_id = ContextSnapshotId::generate(&ids);
        let event_id = tracepress_core::EventId::generate(&ids);
        let batch = begin_context_batch(&BeginContextBatchInput {
            snapshot_id,
            event_id,
            session_id,
            provider_request_id,
            inference_operation_id,
            analysis_version,
            started_at_us,
        });
        let _receipt = self.writer.submit_batch(batch).await?;
        let _replaced = analyses.insert(
            snapshot_id,
            ActiveContextAnalysis {
                session_id,
                provider_request_id,
                inference_operation_id,
                analysis_version,
                started_at_us,
                next_sequence: 0,
                next_ordinal: 0,
                occurrences: HashMap::new(),
            },
        );
        drop(ids);
        drop(analyses);
        drop(sessions);
        Ok(snapshot_id)
    }

    /// Appends one ordered context block batch through the single writer.
    ///
    /// # Errors
    /// Returns a sequence, context-limit, block-validation, unknown-snapshot, or storage error
    /// when the append cannot be admitted and durably recorded.
    pub async fn append_context_blocks(
        &self,
        batch: ContextBlockBatch,
    ) -> Result<ContextAppendReceipt, DaemonError> {
        let mut analyses = self.context_analyses.lock().await;
        let active =
            analyses
                .get_mut(&batch.snapshot_id)
                .ok_or(DaemonError::UnknownContextSnapshot {
                    snapshot_id: batch.snapshot_id,
                })?;
        let final_ordinal = validate_append_batch(active, &batch)?;
        let ids = self.ids.lock().await;
        let (write_batch, allocated) = context_block_batch_commands(active, &batch, &ids)?;
        drop(ids);
        let _receipt = self.writer.submit_batch(write_batch).await?;
        for (ordinal, occurrence_id) in allocated {
            let _replaced = active.occurrences.insert(ordinal, occurrence_id);
        }
        active.next_ordinal = final_ordinal;
        active.next_sequence = active.next_sequence.saturating_add(1);
        let receipt = ContextAppendReceipt {
            accepted_block_count: u64::from(active.next_ordinal),
            next_sequence: active.next_sequence,
            capacity: append_capacity(active),
            status: AnalyzerAnalysisStatus::Partial,
        };
        drop(analyses);
        Ok(receipt)
    }
    /// Aborts one active context analysis and durably records its typed drop reason.
    ///
    /// The active entry is removed only after the snapshot outcome and dropped lifecycle event
    /// commit in one writer batch. A storage failure therefore leaves the analysis retryable.
    ///
    /// # Errors
    /// Returns an unknown-snapshot error for an already terminal analysis or a storage error when
    /// the abort batch cannot commit.
    pub async fn abort_context_analysis(
        &self,
        request: ContextAnalysisAbort,
    ) -> Result<(), DaemonError> {
        let mut analyses = self.context_analyses.lock().await;
        let active =
            analyses
                .get(&request.snapshot_id)
                .ok_or(DaemonError::UnknownContextSnapshot {
                    snapshot_id: request.snapshot_id,
                })?;
        let ids = self.ids.lock().await;
        let write_batch = context_analysis_abort_batch(active, &request, &ids);
        drop(ids);
        let _receipt = self.writer.submit_batch(write_batch).await?;
        let _removed = analyses.remove(&request.snapshot_id);
        drop(analyses);
        Ok(())
    }

    /// Atomically finalizes metrics, deltas, reconciliation, outcome, and events.
    ///
    /// # Errors
    /// Returns an invalid-finalize, unknown-snapshot, or storage error when the terminal payload
    /// cannot be validated and durably recorded.
    pub async fn finalize_context_analysis(
        &self,
        summary: ContextAnalysisFinalize,
    ) -> Result<(), DaemonError> {
        let mut analyses = self.context_analyses.lock().await;
        let active =
            analyses
                .get(&summary.snapshot_id)
                .ok_or(DaemonError::UnknownContextSnapshot {
                    snapshot_id: summary.snapshot_id,
                })?;
        validate_finalize(active, &summary)?;
        let ids = self.ids.lock().await;
        let mut commands = finalize_commands(active, &summary, &ids).into_iter();
        drop(ids);
        let Some(first) = commands.next() else {
            return Err(DaemonError::InvalidContextFinalize {
                snapshot_id: summary.snapshot_id,
            });
        };
        let write_batch = commands.fold(WriteBatch::new(first), WriteBatch::and);
        let _receipt = self.writer.submit_batch(write_batch).await?;
        let _removed = analyses.remove(&summary.snapshot_id);
        drop(analyses);
        Ok(())
    }
}

struct BeginContextBatchInput {
    snapshot_id: ContextSnapshotId,
    event_id: tracepress_core::EventId,
    session_id: SessionId,
    provider_request_id: RequestId,
    inference_operation_id: OperationId,
    analysis_version: u32,
    started_at_us: u64,
}

fn begin_context_batch(input: &BeginContextBatchInput) -> WriteBatch {
    let BeginContextBatchInput {
        snapshot_id,
        event_id,
        session_id,
        provider_request_id,
        inference_operation_id,
        analysis_version,
        started_at_us,
    } = *input;
    WriteBatch::new(WriteCommand::ContextSnapshot {
        snapshot_id,
        session_id,
        provider_request_id,
        inference_operation_id,
        analysis_version,
        status: map_analysis_status(AnalyzerAnalysisStatus::Partial),
        started_at_us,
    })
    .and(WriteCommand::Event {
        event_id,
        session_id: Some(session_id),
        operation_id: Some(inference_operation_id),
        timestamp: started_at_us.to_string(),
        event_type: "context.analysis.started".to_owned(),
        payload: context_event_payload(&ContextEventPayload {
            snapshot_id,
            status: "started",
            analysis_version,
            block_count: None,
        }),
        schema_version: "1".to_owned(),
    })
}

fn validate_append_batch(
    active: &ActiveContextAnalysis,
    batch: &ContextBlockBatch,
) -> Result<u32, DaemonError> {
    if batch.sequence != active.next_sequence {
        return Err(DaemonError::ContextSequence {
            snapshot_id: batch.snapshot_id,
            expected: active.next_sequence,
            actual: batch.sequence,
        });
    }
    if batch.blocks.is_empty() {
        return Err(DaemonError::ContextLimit {
            snapshot_id: batch.snapshot_id,
            dimension: "empty batch",
        });
    }
    if active.next_sequence >= u32::try_from(ContextAnalysisLimits::MAX_BATCHES).unwrap_or(u32::MAX)
    {
        return Err(DaemonError::ContextLimit {
            snapshot_id: batch.snapshot_id,
            dimension: "batches",
        });
    }
    let batch_len = u32::try_from(batch.blocks.len()).map_err(|_| DaemonError::ContextLimit {
        snapshot_id: batch.snapshot_id,
        dimension: "blocks",
    })?;
    let final_ordinal =
        active
            .next_ordinal
            .checked_add(batch_len)
            .ok_or(DaemonError::ContextLimit {
                snapshot_id: batch.snapshot_id,
                dimension: "blocks",
            })?;
    if u64::from(final_ordinal) > ContextAnalysisLimits::MAX_BLOCKS {
        return Err(DaemonError::ContextLimit {
            snapshot_id: batch.snapshot_id,
            dimension: "blocks",
        });
    }
    Ok(final_ordinal)
}

fn append_capacity(active: &ActiveContextAnalysis) -> ContextAppendCapacity {
    let blocks_exhausted = u64::from(active.next_ordinal) >= ContextAnalysisLimits::MAX_BLOCKS;
    let batches_exhausted = active.next_sequence
        >= u32::try_from(ContextAnalysisLimits::MAX_BATCHES).unwrap_or(u32::MAX);
    match (blocks_exhausted, batches_exhausted) {
        (false, false) => ContextAppendCapacity::Available,
        (true, false) => ContextAppendCapacity::BlocksExhausted,
        (false, true) => ContextAppendCapacity::BatchesExhausted,
        (true, true) => ContextAppendCapacity::BothExhausted,
    }
}

fn context_block_batch_commands(
    active: &ActiveContextAnalysis,
    batch: &ContextBlockBatch,
    ids: &UuidV7Generator,
) -> Result<(WriteBatch, Vec<(u32, ContextBlockOccurrenceId)>), DaemonError> {
    let mut commands = Vec::with_capacity(batch.blocks.len());
    let mut allocated = Vec::with_capacity(batch.blocks.len());
    for (index, draft) in batch.blocks.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| DaemonError::ContextLimit {
            snapshot_id: batch.snapshot_id,
            dimension: "blocks",
        })?;
        let expected_ordinal =
            active
                .next_ordinal
                .checked_add(index)
                .ok_or(DaemonError::ContextLimit {
                    snapshot_id: batch.snapshot_id,
                    dimension: "blocks",
                })?;
        if draft.ordinal != expected_ordinal {
            return Err(DaemonError::InvalidContextBlock {
                snapshot_id: batch.snapshot_id,
            });
        }
        let parent_id = match draft.parent_ordinal {
            None => None,
            Some(parent_ordinal) if parent_ordinal < draft.ordinal => Some(
                active
                    .occurrences
                    .get(&parent_ordinal)
                    .copied()
                    .or_else(|| {
                        allocated
                            .iter()
                            .find_map(|(ordinal, id)| (*ordinal == parent_ordinal).then_some(*id))
                    })
                    .ok_or(DaemonError::InvalidContextBlock {
                        snapshot_id: batch.snapshot_id,
                    })?,
            ),
            Some(_) => {
                return Err(DaemonError::InvalidContextBlock {
                    snapshot_id: batch.snapshot_id,
                });
            }
        };
        let occurrence_id = ContextBlockOccurrenceId::generate(ids);
        allocated.push((draft.ordinal, occurrence_id));
        commands.push(context_block_command(&ContextBlockCommandInput {
            snapshot_id: batch.snapshot_id,
            occurrence_id,
            parent_id,
            draft,
        }));
    }
    let mut commands = commands.into_iter();
    let Some(first) = commands.next() else {
        return Err(DaemonError::ContextLimit {
            snapshot_id: batch.snapshot_id,
            dimension: "empty batch",
        });
    };
    let write_batch = commands.fold(WriteBatch::new(first), WriteBatch::and);
    Ok((write_batch, allocated))
}

fn finalize_commands(
    active: &ActiveContextAnalysis,
    summary: &ContextAnalysisFinalize,
    ids: &UuidV7Generator,
) -> Vec<WriteCommand> {
    let mut commands = vec![
        metrics_command(summary.snapshot_id, &summary.metrics),
        reconciliation_command(summary),
    ];
    if let Some(delta) = summary.delta.as_ref() {
        commands.push(delta_command(delta));
    }
    if !matches!(
        summary.visibility.logical_context_status(),
        LogicalContextStatus::ExplicitOnly
    ) {
        commands.push(visibility_event(active, summary, ids));
    }
    commands.push(reconciliation_event(active, summary, ids));
    commands.push(outcome_command(summary));
    commands.push(terminal_event(active, summary, ids));
    if matches!(
        summary.correlation_status,
        ContextCorrelationStatusWire::Degraded
    ) {
        commands.push(correlation_event(active, summary, ids));
    }
    commands
}

fn visibility_event(
    active: &ActiveContextAnalysis,
    summary: &ContextAnalysisFinalize,
    ids: &UuidV7Generator,
) -> WriteCommand {
    WriteCommand::Event {
        event_id: tracepress_core::EventId::generate(ids),
        session_id: Some(active.session_id),
        operation_id: Some(active.inference_operation_id),
        timestamp: summary.completed_at_us.to_string(),
        event_type: "context.visibility.partial".to_owned(),
        payload: context_event_payload(&ContextEventPayload {
            snapshot_id: summary.snapshot_id,
            status: "partial",
            analysis_version: active.analysis_version,
            block_count: None,
        }),
        schema_version: "1".to_owned(),
    }
}

fn reconciliation_event(
    active: &ActiveContextAnalysis,
    summary: &ContextAnalysisFinalize,
    ids: &UuidV7Generator,
) -> WriteCommand {
    let event_type = match summary.reconciliation.comparability {
        AnalyzerReconciliationStatus::ComparableApproximate
        | AnalyzerReconciliationStatus::PartialVisibility => "context.reconciliation.completed",
        AnalyzerReconciliationStatus::MissingProviderUsage
        | AnalyzerReconciliationStatus::MissingLocalEstimate
        | AnalyzerReconciliationStatus::NotComparable
        | _ => "context.reconciliation.unavailable",
    };
    WriteCommand::Event {
        event_id: tracepress_core::EventId::generate(ids),
        session_id: Some(active.session_id),
        operation_id: Some(active.inference_operation_id),
        timestamp: summary.completed_at_us.to_string(),
        event_type: event_type.to_owned(),
        payload: context_event_payload(&ContextEventPayload {
            snapshot_id: summary.snapshot_id,
            status: reconciliation_status_text(summary.reconciliation.comparability),
            analysis_version: active.analysis_version,
            block_count: None,
        }),
        schema_version: "1".to_owned(),
    }
}

fn terminal_event(
    active: &ActiveContextAnalysis,
    summary: &ContextAnalysisFinalize,
    ids: &UuidV7Generator,
) -> WriteCommand {
    WriteCommand::Event {
        event_id: tracepress_core::EventId::generate(ids),
        session_id: Some(active.session_id),
        operation_id: Some(active.inference_operation_id),
        timestamp: summary.completed_at_us.to_string(),
        event_type: if matches!(summary.status, AnalyzerAnalysisStatus::Complete) {
            "context.analysis.completed"
        } else {
            "context.analysis.partial"
        }
        .to_owned(),
        payload: context_event_payload(&ContextEventPayload {
            snapshot_id: summary.snapshot_id,
            status: context_status_text(summary.status),
            analysis_version: active.analysis_version,
            block_count: summary.explicit_block_count,
        }),
        schema_version: "1".to_owned(),
    }
}

fn correlation_event(
    active: &ActiveContextAnalysis,
    summary: &ContextAnalysisFinalize,
    ids: &UuidV7Generator,
) -> WriteCommand {
    WriteCommand::Event {
        event_id: tracepress_core::EventId::generate(ids),
        session_id: Some(active.session_id),
        operation_id: Some(active.inference_operation_id),
        timestamp: summary.completed_at_us.to_string(),
        event_type: "context.correlation.degraded".to_owned(),
        payload: context_event_payload(&ContextEventPayload {
            snapshot_id: summary.snapshot_id,
            status: "degraded",
            analysis_version: active.analysis_version,
            block_count: None,
        }),
        schema_version: "1".to_owned(),
    }
}

fn validate_finalize(
    active: &ActiveContextAnalysis,
    summary: &ContextAnalysisFinalize,
) -> Result<(), DaemonError> {
    if summary.reconciliation.snapshot_id != summary.snapshot_id {
        return Err(DaemonError::ContextFinalizeMismatch {
            snapshot_id: summary.snapshot_id,
        });
    }
    if let Some(delta) = summary.delta.as_ref() {
        if delta.current_snapshot_id != summary.snapshot_id
            || delta.previous_snapshot_id == summary.snapshot_id
        {
            return Err(DaemonError::ContextFinalizeMismatch {
                snapshot_id: summary.snapshot_id,
            });
        }
    }
    if let Some(count) = summary.explicit_block_count {
        if count != u64::from(active.next_ordinal) {
            return Err(DaemonError::InvalidContextFinalize {
                snapshot_id: summary.snapshot_id,
            });
        }
    }
    if matches!(summary.status, AnalyzerAnalysisStatus::Complete)
        && matches!(
            summary.correlation_status,
            ContextCorrelationStatusWire::Degraded
        )
    {
        return Err(DaemonError::InvalidContextFinalize {
            snapshot_id: summary.snapshot_id,
        });
    }
    validate_metrics(&summary.metrics).map_err(|()| DaemonError::InvalidContextFinalize {
        snapshot_id: summary.snapshot_id,
    })
}

fn validate_metrics(metrics: &ContextAnalysisMetrics) -> Result<(), ()> {
    if metrics
        .opportunity_signals
        .as_ref()
        .is_some_and(|signals| signals.len() > 8)
    {
        return Err(());
    }
    for value in [
        metrics.estimated_tool_definition_share,
        metrics.estimated_tool_result_share,
        metrics.estimated_human_text_share,
        metrics.estimated_assistant_history_share,
        metrics.estimated_unique_content_share,
        metrics.estimated_repeated_content_share,
    ]
    .into_iter()
    .flatten()
    {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(());
        }
    }
    Ok(())
}

struct ContextBlockCommandInput<'draft> {
    snapshot_id: ContextSnapshotId,
    occurrence_id: ContextBlockOccurrenceId,
    parent_id: Option<ContextBlockOccurrenceId>,
    draft: &'draft ContextBlockDraft,
}

struct ContextBlockMetadata {
    semantic_path: Option<String>,
    semantic_path_truncated: Option<bool>,
    semantic_path_hash: Option<Box<[u8]>>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    tool_name_truncated: Option<bool>,
    tool_name_hash: Option<Box<[u8]>>,
    semantic_fingerprint: Option<Box<[u8]>>,
    fingerprint_version: Option<u32>,
    estimated_tokens: Option<u64>,
    estimator: Option<String>,
    estimator_version: Option<u32>,
    estimator_encoding: Option<String>,
    estimate_confidence: Option<EstimateConfidence>,
    detected_kind: Option<DetectedContentKind>,
    detector_confidence: Option<f64>,
    detector_version: Option<u32>,
}

fn context_block_metadata(draft: &ContextBlockDraft) -> ContextBlockMetadata {
    let (semantic_path, semantic_path_truncated, semantic_path_hash) =
        metadata(&draft.locator.semantic_path);
    let (tool_call_id, _, _) = optional_metadata(draft.tool_call_id.as_ref());
    let (tool_name, tool_name_truncated, tool_name_hash) =
        optional_metadata(draft.tool_name.as_ref());
    let (semantic_fingerprint, fingerprint_version) =
        draft
            .semantic_fingerprint
            .as_ref()
            .map_or((None, None), |fingerprint| {
                (
                    Some(fingerprint.digest.as_bytes().to_vec().into_boxed_slice()),
                    Some(fingerprint.fingerprint_version),
                )
            });
    let (estimated_tokens, estimator, estimator_version, estimator_encoding, estimate_confidence) =
        draft
            .token_estimate
            .as_ref()
            .map_or((None, None, None, None, None), |estimate| {
                let (estimator, _, _) = metadata(&estimate.estimator);
                let encoding = estimate
                    .encoding
                    .as_ref()
                    .map(|value| value.as_str().to_owned());
                (
                    Some(estimate.tokens),
                    estimator,
                    Some(estimate.estimator_version),
                    encoding,
                    Some(map_estimate_confidence(estimate.confidence)),
                )
            });
    let (detected_kind, detector_confidence, detector_version) = draft
        .detection_result
        .as_ref()
        .map_or((None, None, None), |detection| {
            (
                Some(map_detected_kind(detection.kind)),
                Some(match detection.confidence {
                    DetectionConfidence::Low => 1.0 / 3.0,
                    DetectionConfidence::Medium => 2.0 / 3.0,
                    DetectionConfidence::High => 1.0,
                    _ => 0.0,
                }),
                Some(detection.detector_version),
            )
        });
    ContextBlockMetadata {
        semantic_path,
        semantic_path_truncated,
        semantic_path_hash,
        tool_call_id,
        tool_name,
        tool_name_truncated,
        tool_name_hash,
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
    }
}

struct ContextBlockFeatures {
    line_count: Option<u64>,
    max_line_length: Option<u64>,
    duplicate_line_ratio: Option<f64>,
    unique_line_ratio: Option<f64>,
    json_item_count: Option<u64>,
    json_depth: Option<u64>,
    error_line_density: Option<f64>,
    warning_line_density: Option<f64>,
    repetition_score: Option<f64>,
    opportunity_signals: Option<Box<[OpportunitySignal]>>,
    candidate_estimated_tokens: Option<u64>,
}

fn context_block_features(draft: &ContextBlockDraft) -> ContextBlockFeatures {
    let Some(features) = draft.features.as_ref() else {
        return ContextBlockFeatures {
            line_count: None,
            max_line_length: None,
            duplicate_line_ratio: None,
            unique_line_ratio: None,
            json_item_count: None,
            json_depth: None,
            error_line_density: None,
            warning_line_density: None,
            repetition_score: None,
            opportunity_signals: None,
            candidate_estimated_tokens: None,
        };
    };
    ContextBlockFeatures {
        line_count: features.line_count,
        max_line_length: features.max_line_bytes,
        duplicate_line_ratio: ratio(features.duplicate_line_ratio),
        unique_line_ratio: ratio(features.unique_line_ratio),
        json_item_count: features.json_item_count,
        json_depth: features.json_depth.map(u64::from),
        error_line_density: ratio(features.error_line_density),
        warning_line_density: ratio(features.warning_line_density),
        repetition_score: ratio(features.repetition_score),
        opportunity_signals: (!draft.opportunity_signals.is_empty()).then(|| {
            draft
                .opportunity_signals
                .iter()
                .map(map_opportunity_signal)
                .collect::<Vec<_>>()
                .into_boxed_slice()
        }),
        candidate_estimated_tokens: draft.opportunity_signals.candidate_estimated_tokens(),
    }
}

fn context_block_command(input: &ContextBlockCommandInput<'_>) -> WriteCommand {
    let ContextBlockCommandInput {
        snapshot_id,
        occurrence_id,
        parent_id,
        draft,
    } = *input;
    let ContextBlockMetadata {
        semantic_path,
        semantic_path_truncated,
        semantic_path_hash,
        tool_call_id,
        tool_name,
        tool_name_truncated,
        tool_name_hash,
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
    } = context_block_metadata(draft);
    let ContextBlockFeatures {
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
    } = context_block_features(draft);
    WriteCommand::ContextBlockOccurrence {
        block_occurrence_id: occurrence_id,
        snapshot_id,
        ordinal: u64::from(draft.ordinal),
        parent_block_occurrence_id: parent_id,
        kind: map_block_kind(draft.kind),
        role: draft.role.map_or(ContextRole::Unknown, map_role),
        origin: map_origin(draft.origin),
        semantic_path,
        semantic_path_truncated,
        semantic_path_hash,
        raw_value_start: draft.locator.raw_value_start,
        raw_value_end: draft.locator.raw_value_end,
        locator_occurrence: u64::from(draft.locator.occurrence),
        raw_bytes: draft.raw_bytes,
        exact_fingerprint: Some(
            draft
                .exact_fingerprint
                .as_bytes()
                .to_vec()
                .into_boxed_slice(),
        ),
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
    }
}

fn metrics_command(
    snapshot_id: ContextSnapshotId,
    metrics: &ContextAnalysisMetrics,
) -> WriteCommand {
    let by_kind = estimated_tokens_by_kind(&metrics.estimated_tokens_by_kind);
    let by_role = estimated_tokens_by_role(&metrics.estimated_tokens_by_role);
    let by_origin = estimated_tokens_by_origin(&metrics.estimated_tokens_by_origin);
    WriteCommand::ContextAnalysisMetrics {
        snapshot_id,
        explicit_bytes: metrics.explicit_bytes,
        unknown_block_count: metrics.unknown_block_count,
        semantic_coverage_basis_points: metrics.semantic_coverage_basis_points,
        estimated_tokens: Box::new(EstimatedTokenComposition::new(by_kind, by_role, by_origin)),
        estimated_tool_definition_share: metrics.estimated_tool_definition_share,
        estimated_tool_result_share: metrics.estimated_tool_result_share,
        estimated_human_text_share: metrics.estimated_human_text_share,
        estimated_assistant_history_share: metrics.estimated_assistant_history_share,
        estimated_unique_content_share: metrics.estimated_unique_content_share,
        estimated_repeated_content_share: metrics.estimated_repeated_content_share,
        tool_count: metrics.tool_count,
        schema_bytes: metrics.schema_bytes,
        estimated_schema_tokens: metrics.estimated_schema_tokens,
        largest_tool_schema: metrics.largest_tool_schema,
        repeated_schema_tokens: metrics.repeated_schema_tokens,
        stable_explicit_prefix_estimate: metrics.stable_explicit_prefix_estimate,
        estimator: metrics.estimator.clone(),
        estimator_version: metrics.estimator_version,
        estimate_confidence: metrics.estimate_confidence.map(map_estimate_confidence),
        opportunity_signals: metrics.opportunity_signals.as_ref().map(|signals| {
            signals
                .iter()
                .copied()
                .map(map_opportunity_signal)
                .collect::<Vec<_>>()
                .into_boxed_slice()
        }),
    }
}

fn estimated_tokens_by_kind(values: &[Option<u64>; 15]) -> EstimatedTokensByKind {
    const KINDS: [AnalyzerBlockKind; 15] = [
        AnalyzerBlockKind::Instructions,
        AnalyzerBlockKind::Message,
        AnalyzerBlockKind::Text,
        AnalyzerBlockKind::ImageReference,
        AnalyzerBlockKind::FileReference,
        AnalyzerBlockKind::ToolDefinition,
        AnalyzerBlockKind::ToolCall,
        AnalyzerBlockKind::ToolResult,
        AnalyzerBlockKind::ItemReference,
        AnalyzerBlockKind::PromptReference,
        AnalyzerBlockKind::ProviderStateReference,
        AnalyzerBlockKind::AssistantHistory,
        AnalyzerBlockKind::OpaqueReasoning,
        AnalyzerBlockKind::Opaque,
        AnalyzerBlockKind::Unknown,
    ];
    KINDS.into_iter().zip(values.iter().copied()).fold(
        EstimatedTokensByKind::new(),
        |totals, (kind, value)| {
            value.map_or(totals, |value| totals.with(map_block_kind(kind), value))
        },
    )
}

fn estimated_tokens_by_role(values: &[Option<u64>; 6]) -> EstimatedTokensByRole {
    const ROLES: [AnalyzerRole; 6] = [
        AnalyzerRole::System,
        AnalyzerRole::Developer,
        AnalyzerRole::User,
        AnalyzerRole::Assistant,
        AnalyzerRole::Tool,
        AnalyzerRole::Unknown,
    ];
    ROLES
        .into_iter()
        .zip(values.iter().copied())
        .fold(EstimatedTokensByRole::new(), |totals, (role, value)| {
            value.map_or(totals, |value| totals.with(map_role(role), value))
        })
}

fn estimated_tokens_by_origin(values: &[Option<u64>; 8]) -> EstimatedTokensByOrigin {
    const ORIGINS: [AnalyzerOrigin; 8] = [
        AnalyzerOrigin::HumanAuthored,
        AnalyzerOrigin::AgentGenerated,
        AnalyzerOrigin::ToolGenerated,
        AnalyzerOrigin::ToolSchema,
        AnalyzerOrigin::ProviderManaged,
        AnalyzerOrigin::ExternalReference,
        AnalyzerOrigin::TracepressGenerated,
        AnalyzerOrigin::Unknown,
    ];
    ORIGINS.into_iter().zip(values.iter().copied()).fold(
        EstimatedTokensByOrigin::new(),
        |totals, (origin, value)| {
            value.map_or(totals, |value| totals.with(map_origin(origin), value))
        },
    )
}
fn outcome_command(summary: &ContextAnalysisFinalize) -> WriteCommand {
    WriteCommand::ContextSnapshotOutcome {
        snapshot_id: summary.snapshot_id,
        status: map_analysis_status(summary.status),
        completed_at_us: Some(summary.completed_at_us),
        request_content_hash: summary
            .request_content_hash
            .map(|hash| hash.as_bytes().to_vec().into_boxed_slice()),
        analysis_content_hash: summary
            .analysis_content_hash
            .map(|hash| hash.as_bytes().to_vec().into_boxed_slice()),
        explicit_block_count: summary.explicit_block_count,
        analyzed_bytes: summary.analyzed_bytes,
        skipped_bytes: summary.skipped_bytes,
        explicit_request_complete: Some(summary.visibility.explicit_request_complete),
        uses_previous_response: Some(summary.visibility.uses_previous_response),
        uses_conversation_state: Some(summary.visibility.uses_conversation_state),
        uses_item_references: Some(summary.visibility.uses_item_references),
        uses_prompt_reference: Some(summary.visibility.uses_prompt_reference),
        uses_external_files: Some(summary.visibility.uses_external_files),
        uses_external_images: Some(summary.visibility.uses_external_images),
        contains_opaque_items: Some(summary.visibility.contains_opaque_items),
        logical_context_status: Some(map_logical_status(
            summary.visibility.logical_context_status(),
        )),
        duplicate_key_detected: summary.duplicate_key_detected,
        reference_resolved_locally: summary.reference_resolved_locally,
        correlation_status: Some(map_correlation_status(summary.correlation_status)),
    }
}

const fn delta_command(delta: &ContextDelta) -> WriteCommand {
    WriteCommand::ContextDelta {
        current_snapshot_id: delta.current_snapshot_id,
        previous_snapshot_id: delta.previous_snapshot_id,
        repeated_blocks: Some(delta.repeated_blocks),
        new_blocks: Some(delta.new_blocks),
        changed_blocks: Some(delta.changed_blocks),
        removed_blocks: Some(delta.removed_blocks),
        repeated_estimated_tokens: delta.repeated_estimated_tokens,
        new_estimated_tokens: delta.new_estimated_tokens,
        common_prefix_blocks: delta.common_prefix_blocks,
        common_prefix_estimated_tokens: delta.common_prefix_estimated_tokens,
    }
}

const fn reconciliation_command(summary: &ContextAnalysisFinalize) -> WriteCommand {
    WriteCommand::TokenReconciliation {
        snapshot_id: summary.reconciliation.snapshot_id,
        attempt_id: summary.attempt_id,
        visible_estimated_tokens: summary.reconciliation.visible_estimated_tokens,
        provider_input_tokens: summary.reconciliation.provider_input_tokens,
        residual_tokens: summary.reconciliation.residual_tokens,
        comparability: map_reconciliation_status(summary.reconciliation.comparability),
    }
}

fn metadata(value: &BoundedMetadataText) -> (Option<String>, Option<bool>, Option<Box<[u8]>>) {
    (
        Some(value.as_str().to_owned()),
        Some(value.is_truncated()),
        value
            .full_value_hash()
            .map(|hash| hash.as_bytes().to_vec().into_boxed_slice()),
    )
}

fn optional_metadata(
    value: Option<&BoundedMetadataText>,
) -> (Option<String>, Option<bool>, Option<Box<[u8]>>) {
    value.map_or((None, None, None), metadata)
}

fn ratio(value: Option<FeatureRatio>) -> Option<f64> {
    value.map(FeatureRatio::as_f64)
}

struct ContextEventPayload<'status> {
    snapshot_id: ContextSnapshotId,
    status: &'status str,
    analysis_version: u32,
    block_count: Option<u64>,
}

fn context_event_payload(input: &ContextEventPayload<'_>) -> Box<[u8]> {
    let ContextEventPayload {
        snapshot_id,
        status,
        analysis_version,
        block_count,
    } = *input;
    let block_count = block_count.map_or_else(|| "null".to_owned(), |value| value.to_string());
    format!(
        r#"{{"snapshot_id":"{snapshot_id}","status":"{status}","analysis_version":{analysis_version},"explicit_block_count":{block_count}}}"#
    )
    .into_bytes()
    .into_boxed_slice()
}

const fn context_status_text(status: AnalyzerAnalysisStatus) -> &'static str {
    match status {
        AnalyzerAnalysisStatus::Complete => "complete",
        AnalyzerAnalysisStatus::ResourceLimit => "resource_limit",
        AnalyzerAnalysisStatus::Malformed => "malformed",
        AnalyzerAnalysisStatus::ObserverBackpressure => "observer_backpressure",
        AnalyzerAnalysisStatus::CorrelationDegraded => "correlation_degraded",
        AnalyzerAnalysisStatus::Unsupported => "unsupported",
        AnalyzerAnalysisStatus::Cancelled => "cancelled",
        _ => "unknown",
    }
}
const fn reconciliation_status_text(status: AnalyzerReconciliationStatus) -> &'static str {
    match status {
        AnalyzerReconciliationStatus::ComparableApproximate => "comparable_approximate",
        AnalyzerReconciliationStatus::PartialVisibility => "partial_visibility",
        AnalyzerReconciliationStatus::MissingProviderUsage => "missing_provider_usage",
        AnalyzerReconciliationStatus::MissingLocalEstimate => "missing_local_estimate",
        _ => "unknown",
    }
}

const fn map_block_kind(value: AnalyzerBlockKind) -> ContextBlockKind {
    match value {
        AnalyzerBlockKind::Instructions => ContextBlockKind::Instructions,
        AnalyzerBlockKind::Message => ContextBlockKind::Message,
        AnalyzerBlockKind::Text => ContextBlockKind::Text,
        AnalyzerBlockKind::ImageReference => ContextBlockKind::ImageReference,
        AnalyzerBlockKind::FileReference => ContextBlockKind::FileReference,
        AnalyzerBlockKind::ToolDefinition => ContextBlockKind::ToolDefinition,
        AnalyzerBlockKind::ToolCall => ContextBlockKind::ToolCall,
        AnalyzerBlockKind::ToolResult => ContextBlockKind::ToolResult,
        AnalyzerBlockKind::ItemReference => ContextBlockKind::ItemReference,
        AnalyzerBlockKind::PromptReference => ContextBlockKind::PromptReference,
        AnalyzerBlockKind::ProviderStateReference => ContextBlockKind::ProviderStateReference,
        AnalyzerBlockKind::AssistantHistory => ContextBlockKind::AssistantHistory,
        AnalyzerBlockKind::OpaqueReasoning => ContextBlockKind::OpaqueReasoning,
        AnalyzerBlockKind::Opaque => ContextBlockKind::Opaque,
        _ => ContextBlockKind::Unknown,
    }
}

const fn map_role(value: AnalyzerRole) -> ContextRole {
    match value {
        AnalyzerRole::System => ContextRole::System,
        AnalyzerRole::Developer => ContextRole::Developer,
        AnalyzerRole::User => ContextRole::User,
        AnalyzerRole::Assistant => ContextRole::Assistant,
        AnalyzerRole::Tool => ContextRole::Tool,
        _ => ContextRole::Unknown,
    }
}

const fn map_origin(value: AnalyzerOrigin) -> ContextOrigin {
    match value {
        AnalyzerOrigin::HumanAuthored => ContextOrigin::HumanAuthored,
        AnalyzerOrigin::AgentGenerated => ContextOrigin::AgentGenerated,
        AnalyzerOrigin::ToolGenerated => ContextOrigin::ToolGenerated,
        AnalyzerOrigin::ToolSchema => ContextOrigin::ToolSchema,
        AnalyzerOrigin::ProviderManaged => ContextOrigin::ProviderManaged,
        AnalyzerOrigin::ExternalReference => ContextOrigin::ExternalReference,
        AnalyzerOrigin::TracepressGenerated => ContextOrigin::TracepressGenerated,
        _ => ContextOrigin::Unknown,
    }
}

const fn map_detected_kind(value: AnalyzerDetectedKind) -> DetectedContentKind {
    match value {
        AnalyzerDetectedKind::Json => DetectedContentKind::Json,
        AnalyzerDetectedKind::Ndjson => DetectedContentKind::Ndjson,
        AnalyzerDetectedKind::Log => DetectedContentKind::Log,
        AnalyzerDetectedKind::SearchResults => DetectedContentKind::SearchResults,
        AnalyzerDetectedKind::TestResults => DetectedContentKind::TestResults,
        AnalyzerDetectedKind::SourceCode => DetectedContentKind::SourceCode,
        AnalyzerDetectedKind::Diff => DetectedContentKind::Diff,
        AnalyzerDetectedKind::PlainText => DetectedContentKind::PlainText,
        AnalyzerDetectedKind::BinaryLike => DetectedContentKind::BinaryLike,
        _ => DetectedContentKind::Unknown,
    }
}

const fn map_estimate_confidence(value: AnalyzerEstimateConfidence) -> EstimateConfidence {
    match value {
        AnalyzerEstimateConfidence::ModelMapped => EstimateConfidence::ModelMapped,
        AnalyzerEstimateConfidence::GenericTokenizer => EstimateConfidence::GenericTokenizer,
        _ => EstimateConfidence::Heuristic,
    }
}

const fn map_opportunity_signal(value: AnalyzerOpportunitySignal) -> OpportunitySignal {
    match value {
        AnalyzerOpportunitySignal::LargeToolResult => OpportunitySignal::LargeToolResult,
        AnalyzerOpportunitySignal::HomogeneousJson => OpportunitySignal::HomogeneousJson,
        AnalyzerOpportunitySignal::RepetitiveLogs => OpportunitySignal::RepetitiveLogs,
        AnalyzerOpportunitySignal::LargeSearchResult => OpportunitySignal::LargeSearchResult,
        AnalyzerOpportunitySignal::LargeTestOutput => OpportunitySignal::LargeTestOutput,
        AnalyzerOpportunitySignal::LargeToolSchema => OpportunitySignal::LargeToolSchema,
        AnalyzerOpportunitySignal::RepeatedHistory => OpportunitySignal::RepeatedHistory,
        _ => OpportunitySignal::HighDuplication,
    }
}

const fn map_logical_status(
    value: LogicalContextStatus,
) -> tracepress_storage::LogicalContextStatus {
    match value {
        LogicalContextStatus::ExplicitOnly => {
            tracepress_storage::LogicalContextStatus::ExplicitOnly
        }
        LogicalContextStatus::ProviderManagedPartial => {
            tracepress_storage::LogicalContextStatus::ProviderManagedPartial
        }
        LogicalContextStatus::ExternalReferencesPartial => {
            tracepress_storage::LogicalContextStatus::ExternalReferencesPartial
        }
        LogicalContextStatus::MixedPartial => {
            tracepress_storage::LogicalContextStatus::MixedPartial
        }
        _ => tracepress_storage::LogicalContextStatus::Unknown,
    }
}

const fn map_reconciliation_status(value: AnalyzerReconciliationStatus) -> ReconciliationStatus {
    match value {
        AnalyzerReconciliationStatus::ComparableApproximate => {
            ReconciliationStatus::ComparableApproximate
        }
        AnalyzerReconciliationStatus::PartialVisibility => ReconciliationStatus::PartialVisibility,
        AnalyzerReconciliationStatus::MissingProviderUsage => {
            ReconciliationStatus::MissingProviderUsage
        }
        AnalyzerReconciliationStatus::MissingLocalEstimate => {
            ReconciliationStatus::MissingLocalEstimate
        }
        _ => ReconciliationStatus::NotComparable,
    }
}

const fn map_correlation_status(value: ContextCorrelationStatusWire) -> ContextCorrelationStatus {
    match value {
        ContextCorrelationStatusWire::Correlated => ContextCorrelationStatus::Correlated,
        ContextCorrelationStatusWire::Degraded => ContextCorrelationStatus::Degraded,
    }
}
