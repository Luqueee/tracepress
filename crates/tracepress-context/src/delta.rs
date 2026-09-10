//! Linear, bounded comparison of ordered context block summaries.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracepress_core::ContextSnapshotId;

use crate::{ContextAnalysisLimits, ContextAnalysisStatus, ContextDigest, SemanticFingerprint};

/// Fingerprint and estimate fields needed to compare one ordered block.
#[allow(
    clippy::exhaustive_structs,
    reason = "the summary deliberately requires every identity and missingness input"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextBlockSummary {
    /// Exact identity of the block's raw span bytes.
    pub exact_fingerprint: ContextDigest,
    /// Decoded identity where semantic equivalence was safe to define.
    pub semantic_fingerprint: Option<SemanticFingerprint>,
    /// Local token estimate, absent when the estimator could not produce one.
    pub estimated_tokens: Option<u64>,
}

/// Inputs to one bounded snapshot comparison.
#[allow(
    clippy::exhaustive_structs,
    reason = "the comparison boundary deliberately requires both snapshots and their bound"
)]
#[derive(Clone, Copy, Debug)]
pub struct ContextDeltaRequest<'blocks> {
    /// Earlier snapshot used as the comparison baseline.
    pub previous_snapshot_id: ContextSnapshotId,
    /// Snapshot whose blocks are being classified.
    pub current_snapshot_id: ContextSnapshotId,
    /// Earlier blocks in ordinal order.
    pub previous: &'blocks [ContextBlockSummary],
    /// Current blocks in ordinal order.
    pub current: &'blocks [ContextBlockSummary],
    /// Terminal analysis state of the earlier snapshot.
    ///
    /// A non-`Complete` state means the block list is only an observed prefix or otherwise cannot
    /// support a complete delta, even when its length is exactly `max_blocks`.
    pub previous_analysis_status: ContextAnalysisStatus,
    /// Terminal analysis state of the current snapshot.
    ///
    /// A non-`Complete` state means the block list is only an observed prefix or otherwise cannot
    /// support a complete delta, even when its length is exactly `max_blocks`.
    pub current_analysis_status: ContextAnalysisStatus,
    /// Bounds applied independently to both snapshots.
    pub limits: ContextAnalysisLimits,
}

/// Completeness of a context delta computation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextDeltaStatus {
    /// Both snapshots were compared in full.
    Complete,
    /// At least one snapshot was incomplete or exceeded `max_blocks`; counts cover only the
    /// admitted prefixes.
    ResourceLimit,
}
/// Bounded repetition delta between two snapshots in the same session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ContextDelta {
    /// Earlier snapshot used as the comparison baseline.
    pub previous_snapshot_id: ContextSnapshotId,
    /// Snapshot whose blocks were classified.
    pub current_snapshot_id: ContextSnapshotId,
    /// Whether both inputs were compared completely.
    pub status: ContextDeltaStatus,
    /// Previous blocks admitted to this bounded comparison.
    pub compared_previous_blocks: u64,
    /// Current blocks admitted to this bounded comparison.
    pub compared_current_blocks: u64,
    /// Current blocks matching an available previous exact fingerprint.
    pub repeated_blocks: u64,
    /// Current blocks with no available exact or semantic match.
    pub new_blocks: u64,
    /// Current blocks matching an available previous semantic fingerprint but not exact bytes.
    pub changed_blocks: u64,
    /// Previous blocks left unmatched after exact and semantic multiset matching.
    pub removed_blocks: u64,
    /// Tokens in exact repeated blocks, absent if any such block lacked an estimate.
    pub repeated_estimated_tokens: Option<u64>,
    /// Tokens in genuinely new blocks, absent if any such block lacked an estimate.
    pub new_estimated_tokens: Option<u64>,
    /// Longest leading run with pairwise-equal exact fingerprints; absent if a bound cut the run.
    pub common_prefix_blocks: Option<u64>,
    /// Tokens in the current side of the exact common prefix, preserving unknown estimates.
    pub common_prefix_estimated_tokens: Option<u64>,
}

/// Compares two ordered block lists in linear expected time and bounded memory.
///
/// Exact matches are consumed first as a multiset. Semantic matches classify the remaining current
/// blocks as changed, never as exact repetitions and never as part of the stable prefix.
#[must_use]
pub fn compute_context_delta(request: ContextDeltaRequest<'_>) -> ContextDelta {
    let maximum = request.limits.max_blocks.get();
    let previous_was_limited = request.previous_analysis_status != ContextAnalysisStatus::Complete
        || request.previous.len() > maximum;
    let current_was_limited = request.current_analysis_status != ContextAnalysisStatus::Complete
        || request.current.len() > maximum;
    let previous = request
        .previous
        .get(..request.previous.len().min(maximum))
        .unwrap_or(request.previous);
    let current = request
        .current
        .get(..request.current.len().min(maximum))
        .unwrap_or(request.current);
    compute_bounded_delta(BoundedDeltaRequest {
        previous_snapshot_id: request.previous_snapshot_id,
        current_snapshot_id: request.current_snapshot_id,
        previous,
        current,
        previous_was_limited,
        current_was_limited,
    })
}

#[derive(Clone, Copy)]
struct BoundedDeltaRequest<'blocks> {
    previous_snapshot_id: ContextSnapshotId,
    current_snapshot_id: ContextSnapshotId,
    previous: &'blocks [ContextBlockSummary],
    current: &'blocks [ContextBlockSummary],
    previous_was_limited: bool,
    current_was_limited: bool,
}

fn compute_bounded_delta(request: BoundedDeltaRequest<'_>) -> ContextDelta {
    let BoundedDeltaRequest {
        previous_snapshot_id,
        current_snapshot_id,
        previous,
        current,
        previous_was_limited,
        current_was_limited,
    } = request;
    let mut exact_available = HashMap::<ContextDigest, u64>::with_capacity(previous.len());
    for block in previous {
        increment(exact_available.entry(block.exact_fingerprint).or_default());
    }

    let mut exact_consumed = HashMap::<ContextDigest, u64>::with_capacity(exact_available.len());
    let mut preferred_exact_consumption =
        HashMap::<(ContextDigest, Option<SemanticFingerprint>), u64>::with_capacity(previous.len());
    let mut current_unmatched = Vec::with_capacity(current.len());
    let mut repeated_blocks = 0_u64;
    let mut repeated_tokens = TokenTotal::default();
    for block in current {
        if take_one(&mut exact_available, &block.exact_fingerprint) {
            increment(&mut repeated_blocks);
            increment(exact_consumed.entry(block.exact_fingerprint).or_default());
            increment(
                preferred_exact_consumption
                    .entry((block.exact_fingerprint, block.semantic_fingerprint))
                    .or_default(),
            );
            repeated_tokens.push(block.estimated_tokens);
        } else {
            current_unmatched.push(block);
        }
    }

    let (mut semantic_available, mut remaining_previous) =
        unmatched_previous_semantics(previous, exact_consumed, preferred_exact_consumption);

    let mut changed_blocks = 0_u64;
    let mut new_blocks = 0_u64;
    let mut new_tokens = TokenTotal::default();
    for block in current_unmatched {
        if block
            .semantic_fingerprint
            .is_some_and(|semantic| take_one(&mut semantic_available, &semantic))
        {
            increment(&mut changed_blocks);
            remaining_previous = remaining_previous.saturating_sub(1);
        } else {
            increment(&mut new_blocks);
            new_tokens.push(block.estimated_tokens);
        }
    }

    let (common_prefix_blocks, common_prefix_estimated_tokens) = exact_prefix(
        previous,
        current,
        previous_was_limited || current_was_limited,
    );

    ContextDelta {
        previous_snapshot_id,
        current_snapshot_id,
        status: if previous_was_limited || current_was_limited {
            ContextDeltaStatus::ResourceLimit
        } else {
            ContextDeltaStatus::Complete
        },
        compared_previous_blocks: as_u64(previous.len()),
        compared_current_blocks: as_u64(current.len()),
        repeated_blocks,
        new_blocks,
        changed_blocks,
        removed_blocks: remaining_previous,
        repeated_estimated_tokens: repeated_tokens.finish(),
        new_estimated_tokens: new_tokens.finish(),
        common_prefix_blocks,
        common_prefix_estimated_tokens,
    }
}

fn unmatched_previous_semantics(
    previous: &[ContextBlockSummary],
    mut exact_consumed: HashMap<ContextDigest, u64>,
    mut preferred: HashMap<(ContextDigest, Option<SemanticFingerprint>), u64>,
) -> (HashMap<SemanticFingerprint, u64>, u64) {
    // Prefer the corresponding semantic identity when byte-identical blocks carry different
    // kind/role identities. A second pass consumes any remaining exact occurrence.
    let mut consumed_previous = vec![false; previous.len()];
    for (index, block) in previous.iter().enumerate() {
        let identity = (block.exact_fingerprint, block.semantic_fingerprint);
        if take_one(&mut preferred, &identity) {
            if let Some(consumed) = consumed_previous.get_mut(index) {
                *consumed = true;
            }
            let _ = take_one(&mut exact_consumed, &block.exact_fingerprint);
        }
    }
    for (index, block) in previous.iter().enumerate() {
        if !consumed_previous.get(index).copied().unwrap_or(true)
            && take_one(&mut exact_consumed, &block.exact_fingerprint)
        {
            if let Some(consumed) = consumed_previous.get_mut(index) {
                *consumed = true;
            }
        }
    }

    let mut semantics = HashMap::<SemanticFingerprint, u64>::with_capacity(previous.len());
    let mut remaining = 0_u64;
    for (index, block) in previous.iter().enumerate() {
        if consumed_previous.get(index).copied().unwrap_or(true) {
            continue;
        }
        increment(&mut remaining);
        if let Some(semantic) = block.semantic_fingerprint {
            increment(semantics.entry(semantic).or_default());
        }
    }
    (semantics, remaining)
}

fn exact_prefix(
    previous: &[ContextBlockSummary],
    current: &[ContextBlockSummary],
    possibly_cut: bool,
) -> (Option<u64>, Option<u64>) {
    let mut blocks = 0_u64;
    let mut tokens = TokenTotal::default();
    let mut mismatched = false;
    for (before, after) in previous.iter().zip(current) {
        if before.exact_fingerprint != after.exact_fingerprint {
            mismatched = true;
            break;
        }
        increment(&mut blocks);
        tokens.push(after.estimated_tokens);
    }
    if possibly_cut && !mismatched && previous.len() == current.len() {
        (None, None)
    } else {
        (Some(blocks), tokens.finish())
    }
}

fn take_one<Key>(counts: &mut HashMap<Key, u64>, key: &Key) -> bool
where
    Key: Eq + std::hash::Hash,
{
    let Some(count) = counts.get_mut(key) else {
        return false;
    };
    if *count == 0 {
        return false;
    }
    *count = count.saturating_sub(1);
    true
}
const fn increment(value: &mut u64) {
    *value = value.saturating_add(1);
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[derive(Default)]
struct TokenTotal {
    total: u64,
    unknown: bool,
}

impl TokenTotal {
    const fn push(&mut self, tokens: Option<u64>) {
        match tokens {
            Some(tokens) => self.total = self.total.saturating_add(tokens),
            None => self.unknown = true,
        }
    }

    const fn finish(self) -> Option<u64> {
        if self.unknown { None } else { Some(self.total) }
    }
}
