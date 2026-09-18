# Tracepress Tool Surface Shadow Pilot 014

## Decision

**REJECT an active tool-selection pilot on the measured Codex provider surface.**

The controlled public cohort completed 10 of 10 sessions successfully. Tracepress analyzed all 20
observed provider requests and recorded 10 explicit tool-call blocks, but those requests contained
no explicit tool-definition blocks. The measured opportunity was therefore zero rather than
unknown.

## Evidence

| Metric | Result |
|---|---:|
| Sessions complete and successful | 10/10 |
| Provider requests observed | 20 |
| Complete schema observations | 20 |
| Schema coverage | 100% |
| Tool definitions exposed | 0 |
| Schema bytes | 0 |
| Estimated schema tokens | 0 |
| Repeated schema tokens | 0 |
| Tool-call blocks observed | 10 |
| Provider input tokens | 480,773 |
| Provider cached input tokens | 427,008 |
| Provider uncached input tokens | 53,765 |

Provider usage is observational only. Shadow mode did not remove, rewrite, reorder, or select tools,
so these totals are not savings and are not attributed to a hypothetical policy.

## Instrumentation correction

The first run exposed a zero-value availability bug: a complete analysis with no tool-definition
blocks stored `estimated_schema_tokens` as unavailable while the other schema metrics correctly
stored zero. The producer now records `Some(0)` for this complete known-zero case and retains
`None` for incomplete analysis. A focused regression test establishes the distinction.

The corrected one-session smoke reported full schema coverage and known-zero exposure. The final
N=10 cohort then reproduced the same result across all 20 requests.

## Privacy and safety

- The workload used the pinned public ripgrep repository and commit recorded in the report.
- Only the aggregate allowlisted Tool Surface DTO was persisted.
- No prompts, commands, paths, responses, tool names, identity material, schemas, arguments, or tool
  outputs are present in the report.
- Provider requests and available tools were not mutated.

## Consequence

The result is workload- and integration-scoped. It does not prove that tool-schema optimization is
universally impossible. It does show that building an active selector around this intercepted
surface would optimize zero observed schema material. Phase 6.1 therefore closes without an active
pilot. Reopening requires evidence from a materially different integration or workload where tool
definitions are actually observable before provider execution.
