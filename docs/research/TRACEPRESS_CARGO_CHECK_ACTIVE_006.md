# Tracepress Cargo Check Active 006

## Decision

**ACCEPT `cargo_check_v2_active` for this workload behind explicit opt-in.**

`cargo_check_v1_active` is rejected. Removing the final `Finished` line caused one recovery in its
one-pair success smoke, so no v1 cohort was run. V2 preserves final status, passed a new Shadow
cohort, then passed separate active success and diagnostic fail-open cohorts.

## V2 Shadow prerequisite

| Measurement | Control | Shadow |
|---|---:|---:|
| Sessions / task success | 10 / 10 | 10 / 10 |
| Raw output | 15,070 bytes | 15,070 bytes |
| Candidate output | - | 1,510 bytes |
| Candidate reduction | - | 89.98% |
| Never-worse accepted | - | 10 / 10 |
| Tool calls / retries | 10 / 0 | 10 / 0 |
| Provider requests | 22 | 21 |

Raw remained agent-visible. Provider differences in this prerequisite are A/A noise, not savings.

## Active success result

| Source measurement | Treatment |
|---|---:|
| Executions / mutations | 10 / 10 |
| Raw output | 15,070 bytes |
| Emitted output | 2,300 bytes |
| Source reduction | 84.74% |
| Estimated raw / emitted tokens | 3,770 / 580 |
| Progress lines omitted | 340 |
| Recovery-hint overhead | 1,580 bytes |
| Never-worse accepted | 10 / 10 |
| Reducer latency | 216 us total; 33 us maximum |

| Downstream metric | Control | Treatment | Relative change |
|---|---:|---:|---:|
| Provider requests | 20 | 21 | +5.00% |
| Input total | 483,827 | 502,986 | +3.96% |
| Cached input | 420,864 | 444,160 | +5.53% |
| Uncached input | 62,963 | 58,826 | -6.57% |
| Output | 1,565 | 1,787 | +14.19% |
| Reasoning | 509 | 662 | +30.06% |
| Duration | 111,656 ms | 127,093 ms | +13.83% |

The paired median uncached delta was -476.5 tokens and input-total delta was -491.5. Provider
requests had a zero paired median despite the aggregate +1. Task success was 10/10 in both arms;
each arm made ten tool calls with zero retries and zero recovery requests.

The result is mixed rather than universally cheaper: uncached input improved, but cached input,
output, reasoning, aggregate input, and wall time increased. Acceptance follows the predeclared
gate and remains workload-scoped; it is not a claim of lower total cost for every session.

## Diagnostic fail-open result

Control classified 10/10 failures correctly from 3,070 raw bytes. Treatment also classified 10/10
correctly and never mutated output: every non-applicable evaluation failed open, emitting all raw
bytes. There were zero recoveries. One Treatment session reran the identical command, yielding 11
executions/tool calls versus 10 in Control and one additional provider request. This is the allowed
+1 A/A boundary, not evidence of source savings, because Treatment bytes were raw and identical per
execution.

## Privacy and gate

Temporary worktrees, recovery payloads, provider bodies, prompts, and agent messages were removed
after every arm. Reports retain only allowlisted aggregates and bounded outcome classes.

- final-status preservation: pass;
- Shadow prerequisite: pass;
- active source reduction: pass;
- aggregate and paired-median uncached input: pass;
- objective task quality: pass;
- success retries and recoveries: pass, zero;
- diagnostic raw fail-open: pass;
- diagnostic retry boundary: pass with one explicitly retained rerun;
- privacy and measurement integrity: pass.

Default Tracepress behavior remains passthrough. `cargo clippy` is still unsupported.
