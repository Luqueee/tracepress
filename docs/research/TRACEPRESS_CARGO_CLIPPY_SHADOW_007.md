# Tracepress Cargo Clippy Shadow 007

## Decision

**REJECT `cargo_clippy_v1` for active evaluation on the current workload.**

The paired Shadow smoke was correct but materially ineffective. Per the predeclared gate, no N=10
cohort and no active implementation were run.

## Result

| Measurement | Control | Shadow |
|---|---:|---:|
| Objective task success | 1 / 1 | 1 / 1 |
| Raw output | 136,328 bytes | 136,328 bytes |
| Agent-visible output | 136,328 bytes | 136,328 bytes |
| Candidate output | - | 134,972 bytes |
| Candidate reduction | - | 0.99% |
| Estimated raw tokens | - | 34,082 |
| Estimated candidate tokens | - | 33,743 |
| Progress lines omitted | - | 34 |
| Hypothetical hint overhead | - | 79 bytes |
| Never-worse accepted | - | 1 / 1 |
| Reducer latency | - | 674 us |
| Tool calls / retries | 1 / 0 | 1 / 0 |
| Provider requests | 2 | 2 |

The command emitted a large diagnostic/lint stream. The safe projection removed only Cargo
progress, so 99.01% of bytes remained. Raw output was forwarded exactly, making provider deltas
non-causal A/A observations.

## Interpretation

The result does not show that Clippy output can never be reduced. It shows that progress stripping
alone is not a material source optimization for this workload. Improving the ratio would require a
new policy such as warning grouping, deduplication, or bounded diagnostic projection. Those policies
change semantic presentation and require their own evidence, objective quality tasks, and recovery
design; they are not justified by this smoke.

Reports contain only aggregate allowlisted metadata. No command, arguments, path, prompt, raw
diagnostic content, provider body, session id, source execution id, or recovery token was retained.
