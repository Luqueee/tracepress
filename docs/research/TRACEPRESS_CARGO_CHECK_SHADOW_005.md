# Tracepress Cargo Check Shadow 005

## Decision

**PASS for Phase 5.2 Shadow. READY for a separate `cargo_check_v1` active pilot.**

This accepts one candidate reducer on one pinned public workload. It does not forward filtered
output, enable a default policy, or establish value for `clippy`, `build`, search, or Git.

## Method

Control and Treatment each ran the same absolute `tracepress tool cargo check` command in ten
paired sessions with alternating order. Every arm used a clean detached worktree, an isolated
Cargo target directory, one Cargo job context, the same ripgrep commit, and the same model. No hook
or PATH interception participated.

Treatment evaluated `cargo_check_v1_shadow` but returned raw stdout and stderr. The objective task
was to classify the command outcome using one exact structured sentinel. Only aggregate metadata
and bounded outcome classes entered the report.

## Source result

| Measurement | Treatment result |
|---|---:|
| Executions | 10 |
| Raw output | 15,070 bytes |
| Agent-visible output | 15,070 bytes |
| Candidate output | 790 bytes |
| Candidate reduction | 94.76% |
| Estimated raw tokens | 3,770 |
| Estimated candidate tokens | 200 |
| Never-worse accepted | 10 / 10 |
| Forwarding mutations | 0 |
| Progress lines omitted in candidates | 350 |
| Hypothetical recovery-hint overhead | 790 bytes |
| Reducer latency | 236 us total; 34 us maximum |

The candidate consists only of the hypothetical recovery hint on this successful workload because
all observed command output was recognized Cargo progress. That result is useful but narrow: an
active pilot must separately demonstrate that success remains understandable and that diagnostic
failures stay sufficient without causing retries or recovery.

## Downstream A/A result

| Metric | Control | Shadow | Delta |
|---|---:|---:|---:|
| Provider requests | 20 | 20 | 0 |
| Input total | 482,764 | 483,229 | +465 |
| Cached input | 364,544 | 387,072 | +22,528 |
| Uncached input | 118,220 | 96,157 | -22,063 |
| Output | 1,588 | 1,595 | +7 |
| Reasoning | 524 | 463 | -61 |
| Duration | 116,542 ms | 115,198 ms | -1,344 ms |

Raw agent-visible bytes were identical, so none of these provider deltas is attributed to source
reduction. The paired median input-total delta was -8 tokens, uncached was -43.5, and provider
requests were unchanged.

## Quality, behavior, and privacy

Both arms achieved 10/10 objective task success, 10 tool calls, zero command retries, zero recovery
requests, and zero provider errors. Every Treatment execution linked its source identity to its
provider session and emitted raw bytes exactly.

Experiment state was removed after each arm. Durable artifacts contain no prompt, command,
arguments, path, raw output, provider body, recovery token, or agent message.

## Gate

- command and process contract: pass;
- raw forwarding invariant: pass;
- objective task quality: pass, 10/10 in both arms;
- candidate reduction: pass, 94.76%;
- never-worse: pass, 10/10;
- retry and tool-call regression: pass;
- reducer overhead: pass, 34 us maximum;
- measurement and privacy integrity: pass.

Phase 5.2 therefore authorizes only a separately controlled active pilot for `cargo_check_v1`.
Default Tracepress behavior remains passthrough, and no second new reducer is admitted.
