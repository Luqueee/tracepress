# Tracepress Source Active 004

## Decision

**PASS for the bounded Phase 5.1 `cargo_test_v1` active pilot.**

The result accepts one active reducer for one pinned public workload. It does not enable the policy
by default, establish value for other command families, or justify hook/PATH interception.

## Method

Control and Treatment each ran the same absolute `tracepress tool cargo test` command in ten paired,
session-level assignments. Order alternated by pair. Every arm used a clean detached worktree, its
own Cargo target directory, one test thread, the same ripgrep commit, and the same model.

Control returned raw stdout and stderr. Treatment evaluated `cargo_test_v1`, included the complete
recovery command in never-worse, wrote byte-faithful recovery before mutation, and forwarded the
candidate. No hook participated.

The pinned test command exits nonzero. This is useful failure-focused evidence rather than a task
failure: the objective task was to classify the result. Codex emitted one exact structured sentinel,
`TESTS_PASSED` or `TESTS_FAILED`; Tracepress compared it transiently with the source exit class and
persisted only the bounded outcome classes.

## Source result

| Measurement | Treatment result |
|---|---:|
| Executions | 10 |
| Raw output | 206,740 bytes |
| Agent-visible output | 13,390 bytes |
| Source-output reduction | 93.52% |
| Estimated raw tokens | 51,690 |
| Estimated emitted tokens | 3,350 |
| Never-worse accepted | 10 / 10 |
| Forwarding mutations | 10 / 10 |
| Fail-open executions | 0 |
| Passing test lines omitted | 4,490 |
| Progress lines omitted | 400 |
| Recovery-hint overhead | 1,790 bytes |
| Reducer latency | 727 µs total; 92 µs maximum |
| Source/provider session links | 10 / 10 |

## Downstream result

| Metric | Control | Treatment | Delta | Relative delta |
|---|---:|---:|---:|---:|
| Provider requests | 20 | 21 | +1 | +5.00% |
| Input total | 532,117 | 505,601 | -26,516 | -4.98% |
| Cached input | 430,080 | 440,064 | +9,984 | +2.32% |
| Uncached input | 102,037 | 65,537 | -36,500 | -35.77% |
| Output | 1,669 | 1,782 | +113 | +6.77% |
| Reasoning | 583 | 722 | +139 | +23.84% |
| Duration | 198,031 ms | 186,247 ms | -11,784 ms | -5.95% |

The paired median uncached delta was -5,361.5 tokens. The paired median provider-request delta was
zero, so the aggregate +1 request remains inside the previously measured explicit A/A noise. The
output and reasoning increases are retained as real tradeoffs; the result is not described as a
uniform reduction across every token category.

## Quality and behavior

| Measurement | Control | Treatment |
|---|---:|---:|
| Objective task success | 10 / 10 | 10 / 10 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |
| Recovered bytes | 0 | 0 |
| Provider errors | 0 | 0 |

Treatment retained sufficient failure information for every agent to classify the nonzero result
without recovery or rerunning the command.

## Recovery and privacy

Active recovery uses a random 128-bit opaque token, validated same-session access, 0700 directories,
0600 files, separate byte-faithful stdout/stderr, an 8 MiB bound, and one-hour expiry. Store failure,
missing session, never-worse rejection, and oversized output all fail open to raw output. A local
contract smoke proved byte-exact recall and the missing-session fail-open path.

Experiment states and raw recovery payloads were deleted after each arm. Durable reports contain no
prompt, command, arguments, paths, output content, provider body, or recovery token.

## Gate

- command/process contract: pass;
- objective task quality: pass, 10/10 in both arms;
- source reduction: pass, 93.52%;
- aggregate and paired-median uncached input: pass;
- provider-request noise bound: pass, +1 aggregate and zero paired median;
- retry/tool-call regression: pass;
- recovery-rate gate: pass, 0%;
- fail-open integrity: pass;
- measurement/privacy integrity: pass.

`cargo_test_v1_active` is accepted for this workload behind its explicit opt-in environment policy.
Default Tracepress behavior remains passthrough. Expansion to `cargo check` or any other family
requires a new isolated reducer decision.
