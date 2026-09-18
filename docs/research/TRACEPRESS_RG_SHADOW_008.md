# Tracepress RG Shadow 008

## Decision

**PASS for `rg_v1` Shadow. READY for a separate active pilot.**

This accepts lossless file grouping for one pinned public workload. It does not authorize capping,
line truncation, lossy deduplication, default activation, or any `git status` reducer.

## Fail-closed preflight

The initial `rg -n fn crates` smoke produced 228,262 bytes across 2,981 lines. Shape-only transient
analysis found 2,977 unambiguous lines and four lines with multiple `:digits:` boundaries. The
reducer treated the complete stream as non-applicable, returned a 228,262-byte raw-equivalent
candidate, and preserved task success. No raw content entered the report.

## Measured workload

The measured command was `rg -n struct crates` on the same pinned public ripgrep commit used by the
other source experiments. Control and Treatment each ran ten sessions with alternating order, clean
detached worktrees, isolated state, and an exact structured outcome sentinel.

| Source measurement | Treatment result |
|---|---:|
| Executions | 10 |
| Raw output | 288,780 bytes |
| Agent-visible output | 288,780 bytes |
| Candidate output | 198,720 bytes |
| Candidate reduction | 31.19% |
| Estimated raw tokens | 72,200 |
| Estimated candidate tokens | 49,680 |
| Match lines grouped | 4,150 |
| Never-worse accepted | 10 / 10 |
| Recovery-hint overhead | 0 bytes |
| Reducer latency | 5,161 us total; 770 us maximum |

Grouping is lossless, so no recovery hint is required in Shadow. Every Treatment execution emitted
the original 28,878 bytes.

## Quality and downstream A/A

Both arms achieved 10/10 objective task success, ten tool calls, zero retries, zero recoveries, zero
provider errors, and twenty provider requests. The paired median provider-request delta was zero.

Control input was 532,697 total, 358,400 cached, and 174,297 uncached tokens. Shadow input was
538,511 total, 319,488 cached, and 219,023 uncached. These differences are retained as trajectory
noise and are not attributed to grouping because agent-visible bytes were identical.

## Privacy and next gate

Reports contain only aggregate allowlisted measurements. They contain no command, arguments, paths,
matches, prompt, agent message, provider body, session id, source execution id, or recovery token.

An active pilot must include both the parseable workload and an ambiguous-output safety workload.
The latter must fail open without mutation. Source reduction alone is insufficient: active success
also requires lower uncached input, unchanged task quality, no material retry/tool-call increase,
and low recovery.
