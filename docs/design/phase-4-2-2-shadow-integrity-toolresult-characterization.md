# Phase 4.2.2 — Shadow Integrity & ToolResult Characterization

Phase 4.2.2 keeps forwarding byte-exact and does not enable request rewriting. It
closes two measurement gaps from Shadow Pilot 003 before another efficacy claim is
made:

1. distinguish analysis-level shadow jobs from candidate evaluations;
2. characterize context by origin, block kind, and detected content kind before
   selecting another provider-compatible target.

## Scheduling finding

The current scheduler already admits **one `ShadowJob` per finalized context
analysis**. The worker evaluates the bounded compressor set inside that job. Queue
pressure is therefore proportional to admitted analyses, not to
`eligible_blocks × compressors`. The previous pilot's 24 drops must not be
reported as 24 queue jobs per candidate.

The additive v12 counters make this distinction durable:

```text
shadow_jobs_admitted
shadow_jobs_processed
shadow_job_drops
candidate_evaluations_attempted
candidate_evaluations_completed
candidate_evaluation_drops
```

Existing reason counters remain separate. `shadow_drops` is the aggregate health
counter; `shadow_job_drops` counts jobs rejected before worker processing, while
`candidate_evaluation_drops` counts compressor evaluations skipped by the
per-job work budget. A persistence failure is a shadow health drop but not a
second job drop.

The bounded worker keeps the shadow queue and byte budget independent from
forwarding and context-analysis queues. No counter or migration in this phase
changes the forwarding path.

## Controlled integrity gate

The next controlled smoke must require:

```text
shadow_jobs_admitted == shadow_jobs_processed
shadow_job_drops == 0
candidate_evaluation_drops == 0
candidate_evaluations_attempted == candidate_evaluations_completed
forwarding_mutations == 0
recovery_failures == 0
determinism_failures == 0
```

Naturalistic runs may be marked degraded when a bounded budget is intentionally
exhausted; they must not be used as an efficacy gate without reporting the
missingness.

## Origin × kind characterization

`scripts/characterize_provider_shapes.py` now emits a metadata-only matrix for
the selected sessions:

```text
ContextOrigin × ContextBlockKind × DetectedContentKind
```

For every cell it reports block count, estimated tokens, raw bytes, token share,
and plain-text duplicate-line/line-count aggregates where available. It never
selects raw bytes, payloads, tool arguments, responses, URLs, or fingerprints.

This matrix is required before a PlainText target is prioritized. Global
PlainText share is not evidence that `ToolGenerated / ToolResult / PlainText`
is present: the characterization probe found PlainText in human-authored text,
while the ToolResult PlainText target remained empty.

## Decision gate

Run a directed naturalistic cohort only if the matrix shows material ToolResult
PlainText exposure. A deterministic provider-compatible candidate remains
eligible for Active A/B only when real traffic demonstrates approximately:

```text
addressable token share >= 5%
median reduction >= 10%
or effective context reduction >= 2–3%
```

These are triage thresholds, not provider-token or cost claims. If no current
candidate crosses them after an integrity-clean cohort, the reversible path is
closed as a valid negative result and Phase 4.3 becomes a separate, quality-led
Tool-Aware Context Reduction phase.

## Artifacts

- `reports/provider-compatible-candidates-001/TRACEPRESS_PROVIDER_COMPATIBLE_CHARACTERIZATION_PROBE_002.json`
- `reports/provider-compatible-candidates-001/TRACEPRESS_PROVIDER_COMPATIBLE_CHARACTERIZATION_PROBE_002.md`

The probe is a one-session metadata-only characterization of an existing local
fixture/probe database, not a replacement for the required N=10 directed cohort.
