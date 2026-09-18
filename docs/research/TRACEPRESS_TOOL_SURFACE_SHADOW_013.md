# Tracepress Tool Surface Shadow 013

## Decision

**PASS the Phase 6.0 instrumentation slice. Do not claim tool-selection savings.**

Tracepress can now characterize tool-schema exposure and observed tool usage from existing
metadata through a read-only, aggregate-only Observatory surface. Provider requests remain
byte-for-byte outside this feature's control.

## Evidence boundary

The projection reports:

- latest-snapshot schema observation coverage;
- aggregate tool definitions, schema bytes, estimated schema tokens, and repeated schema tokens;
- explicit tool-call count;
- session-scoped distinct definition/use counts when identity coverage is complete; and
- provider-reported input, cached, uncached, output, and reasoning usage separately.

This is characterization, not a causal experiment. Provider usage covers the same analyzed
provider-request cohort but is not attributed to any definition, tool, or hypothetical selection
policy.

## Safety and privacy

- SQLite is opened read-only with `PRAGMA query_only = ON` by the existing Observatory path.
- No migration, hook, proxy, reducer, or provider adapter is added.
- The API DTO cannot carry tool names, identity material, schemas, arguments, outputs, prompts, paths,
  or request/session identifiers.
- Incomplete schema metrics remain unavailable.
- Schema totals and repetition share use one coherent complete-row cohort.
- Provider usage is restricted to provider requests represented by the latest analyzed snapshots.
- Incomplete snapshots, missing per-request snapshots in an observed session, or incomplete
  definition/call identity coverage make all derived distinct and unused counts unavailable.
- `unused_tools_lower_bound` is descriptive evidence only; absence of a matching explicit call is
  not proof that a tool is globally unnecessary.

## Tests

The focused API tests establish these boundaries:

1. a normal synthetic database returns Shadow metadata and contains none of the forbidden field
   names; and
2. a controlled fixture with complete schema aggregates and ordinary untruncated tool names returns
   the expected exposure, repetition, usage, and unused lower-bound counts; and
3. complementary partial schema rows cannot create a ratio; an incomplete context snapshot or a
   same-session request without a snapshot makes derived definition/use/unused counts unavailable.

The repeated-schema share is computed as repeated estimated tokens divided by total estimated
schema tokens. A test-first fixture caught and corrected the reversed ratio during development.

## Next gate

The next phase is a controlled public Shadow cohort, not active tool removal. It must define a
task-level success evaluator and preserve the complete original tool surface seen by the agent.
Only after exposure, usage, provider behavior, and quality are jointly measurable should an active
selection experiment be designed.
