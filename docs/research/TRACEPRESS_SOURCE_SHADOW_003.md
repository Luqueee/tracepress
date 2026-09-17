# Tracepress Source Shadow 003

## Decision

**PASS for Phase 5.0 Shadow. READY for a separately authorized Phase 5.1 active pilot.**

This decision accepts `cargo_test_v1` as a measured candidate. It does not enable filtered output,
claim provider savings, or approve additional command families.

## Causal surface

Both arms explicitly instructed Codex to invoke the same absolute Tracepress tool command. No hook
or PATH rewrite was active. Control used passthrough; treatment evaluated `cargo_test_v1` in memory
and returned the original stdout and stderr. Runs used the same pinned public ripgrep commit,
per-arm clean worktrees, per-arm Cargo target directories, one test thread, and alternating order.

The preceding explicit A/A completed 10 pairs with 20/20 successful sessions, zero provider errors,
one source execution per session, zero hook rewrites, and identical 20,674 emitted bytes in every
pair. Provider requests were 20 versus 21 with a paired median delta of zero; that variation defines
noise between identical arms.

## Shadow result

The 10-pair Shadow pilot completed 20/20 sessions successfully with zero provider errors. Control
and Shadow each made 20 provider requests. Every pair emitted exactly 20,674 raw bytes in each arm;
there were no forwarding mutations.

Across the 10 Shadow executions:

| Source measurement | Result |
|---|---:|
| Raw output | 206,740 bytes |
| Candidate output | 12,430 bytes |
| Candidate source reduction | 93.99% |
| Estimated raw tokens | 51,690 |
| Estimated candidate tokens | 3,110 |
| Never-worse accepted | 10 / 10 |
| Passing test lines omitted | 4,490 |
| Progress lines omitted | 400 |
| Recovery hint overhead | 830 bytes |
| Reducer time | 810 µs total; 138 µs maximum |
| Source ids present | 10 / 10 |
| Source sessions linked to provider sessions | 10 / 10 |

The source candidate gate passes: reduction was material, all candidates passed never-worse, every
execution retained its source identity and provider-session join, and overhead was bounded.

## Downstream result

| Metric | Control | Shadow | Paired median Shadow − Control |
|---|---:|---:|---:|
| Provider requests | 20 | 20 | 0 |
| Input total | 407,457 | 408,293 | -22 |
| Cached input | 248,832 | 265,216 | 0 |
| Uncached input | 158,625 | 143,077 | -32 |
| Output | 1,724 | 1,649 | -3 |
| Reasoning | 410 | 424 | 4.5 |
| Duration | 190,869 ms | 178,229 ms | -1,755.5 ms |

These are trajectory observations, not reducer savings: agent-visible bytes were raw and identical.
The absence of additional requests and the zero paired-median request delta are consistent with the
explicit A/A envelope. The lower aggregate uncached input cannot be causally attributed to the
candidate while forwarding remains unchanged.

## Recovery and privacy

Shadow stored no recovery payload and advertised no live recovery. The hypothetical 83-byte hint
per execution was included in candidate size and never-worse. Reports contain aggregate allowlisted
metadata only: no prompts, commands, arguments, paths, working directories, stdout, stderr, or
provider bodies.

## Gate outcome

- command correctness: pass for the pinned public workload;
- sandbox/approval preservation: unchanged because Codex selected and executed the explicit command;
- task quality: 20/20 successful sessions;
- fail-open shell admission: covered by proxy tests and not expanded;
- measurement integrity: pass;
- source candidate materiality: pass;
- provider savings: not tested and not claimed;
- retries: zero in the forced single-command workload;
- recoveries: unavailable by design in Shadow;
- breadth: still limited to `cargo test`.

Phase 5.0 is complete once this evidence, the read-only Observatory projection, and the final clean
validation commit are recorded. Phase 5.1 must use session-level Control/Treatment assignment and
may emit a filtered result only behind a new explicit active policy.
