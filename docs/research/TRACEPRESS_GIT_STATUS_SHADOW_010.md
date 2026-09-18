# Tracepress Git Status Shadow 010

## Decision

**PASS for `git_status_v1` Shadow. READY for a separate active pilot.**

This result accepts only removal of Git's human guidance lines for the measured long-form status
shape. It does not authorize active mutation, flags, porcelain rewriting, lossy path handling, or
default activation.

## Measured cohort

Control and Shadow each ran ten sessions on the same pinned public ripgrep commit with alternating
order, isolated detached worktrees, isolated Tracepress state, and identical deterministic dirty
worktree fixtures.

| Source measurement | Shadow result |
|---|---:|
| Executions | 10 |
| Raw output | 4,530 bytes |
| Agent-visible output | 4,530 bytes |
| Candidate output | 2,030 bytes |
| Candidate reduction | 55.19% |
| Advisory lines omitted | 40 |
| Never-worse accepted | 10 / 10 |
| Recovery-hint overhead | 0 bytes |
| Reducer latency | 203 us total; 28 us maximum |

Both arms achieved 10/10 objective task success, ten tool calls, zero retries, zero recoveries, zero
provider errors, and twenty provider requests. Shadow produced zero forwarding mutations.

## Downstream A/A observation

Control provider input was 481,668 total, 377,344 cached, and 104,324 uncached tokens. Shadow input
was 481,610 total, 366,080 cached, and 115,530 uncached. The paired median uncached delta was +24.
These values are retained as A/A trajectory noise: both arms delivered the same 453 source bytes per
session, so the candidate cannot have caused the provider difference.

## Safety and privacy

Unit coverage verifies raw-equivalent abstention for clean, localized, and non-UTF-8 output, plus
detached-HEAD handling and exact stderr preservation. The committed reports contain no command,
arguments, paths, status output, prompts, agent messages, provider bodies, session/source IDs, or
recovery tokens.

An active pilot must count its recovery hint inside never-worse and demonstrate lower uncached input
without quality, retry, tool-call, or recovery regressions. Phase 5.7 itself makes no active claim.
