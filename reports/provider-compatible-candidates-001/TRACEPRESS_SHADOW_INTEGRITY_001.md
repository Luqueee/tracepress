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

The previous N=10 Pilot 003 remains `completed_degraded`: 31 requests, 24
aggregate shadow drops, and no material J5/J6/J7 reduction. That result predates
the v12 job/evaluation counters and is retained as evidence, not reused as an
integrity-clean gate.

The available metadata-only characterization probe found 23,060 global
PlainText estimated tokens in `human_authored` blocks, but zero
`ToolGenerated / ToolResult / PlainText` blocks. Its single ToolResult block was
JSON (95 estimated tokens). Therefore a directed ToolResult PlainText cohort is
not justified by this probe alone.

## Decision

Phase 4.3 remains blocked. Before another candidate efficacy decision, run a
clean N=10 naturalistic cohort with the new counters, or a directed real
ToolResult cohort if the Origin × Kind matrix shows material safe exposure. Do
not infer provider savings, cache preservation, or quality from this report.
