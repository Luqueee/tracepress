# Tracepress Shadow Integrity 001

Phase 4.2.2 implementation and gate status.

## Scheduler finding

The implementation admits one `ShadowJob` per finalized context analysis. The
worker evaluates the bounded compressor set inside that job. Candidate count does
not multiply queue entries.

## Controlled smoke

`shadow_compression_post_hardening_smoke_is_bounded_and_byte_exact` now asserts:

```text
jobs admitted == jobs processed
shadow job drops = 0
candidate evaluations attempted == completed
candidate evaluation drops = 0
forwarding mutations = 0
recovery failures = 0
```

The smoke passed. This validates accounting and byte-exact forwarding for the
bounded local path; it is not an efficacy claim.

## Naturalistic and PlainText status

The regenerated v12 N=10 Pilot 003 is integrity-clean: 10 sessions, 20 provider
requests, 20 jobs admitted/processed, 70 candidate evaluations attempted and
completed, zero job/evaluation drops, zero forwarding mutations, zero recovery
failures, zero determinism failures, and zero Unknown transformations. It still
found zero applicable blocks for the human-readable JSON candidates and no
material reduction. The earlier 31-request / 24-drop run is retained only as a
pre-v12 degraded observation.

The available metadata-only characterization probe found 23,060 global
PlainText estimated tokens in `human_authored` blocks, but zero
`ToolGenerated / ToolResult / PlainText` blocks. Its single ToolResult block was
JSON (95 estimated tokens). Therefore a directed ToolResult PlainText cohort is
not justified by this probe alone.

## Decision

The current deterministic provider-compatible path is closed as a valid negative
result for the candidates evaluated. No active candidate is selected and Phase
4.3 remains blocked as a release gate. If resumed, it must be a separately
designed quality-led Tool-Aware Context Reduction phase, not another sequence of
opaque JSON encodings. Do not infer provider savings, cache preservation, or
quality from this report.
