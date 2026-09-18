# Tracepress Git Status Active 011

## Decision

**REJECT `git_status_v1_active` for the measured workload.**

The reducer remains an explicit experimental path for reproducibility, but it is not promoted as an
accepted policy and default runtime behavior remains passthrough.

## Dirty active cohort

Control and Treatment each ran ten sessions with identical public dirty-worktree fixtures.

| Measurement | Control | Treatment |
|---|---:|---:|
| Task success | 10 / 10 | 10 / 10 |
| Raw source output | 4,530 bytes | 4,530 bytes |
| Agent-visible output | 4,530 bytes | 3,450 bytes |
| Source reduction | 0% | 23.84% |
| Provider input total | 482,120 | 481,936 |
| Provider cached input | 425,984 | 423,936 |
| Provider uncached input | 56,136 | 58,000 |
| Provider output | 1,242 | 1,334 |
| Provider reasoning | 402 | 472 |
| Provider requests | 20 | 20 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |

All ten candidates passed never-worse after including 1,420 total hint bytes. They omitted 40 advice
lines and took 214 microseconds total reducer time. Task quality and trajectory counts remained
stable.

However, aggregate uncached input increased by 1,864 tokens, or 3.32%. The paired
Treatment-minus-Control median was -9.5 tokens, with six negative, three positive, and one zero
pair. Phase 5.8 requires both lower aggregate uncached input and a negative paired median; only the
second condition passed. Source-byte reduction therefore does not qualify as downstream value.

## Clean fail-open cohort

The independent clean cohort passed its safety gate. Treatment produced ten active evaluations,
ten fail-opens, zero forwarding mutations, zero recovery objects or requests, zero retries, ten tool
calls, twenty provider requests, and 10/10 objective task success. Every session emitted the exact
67 raw bytes.

Provider differences in the clean cohort are A/A noise because both arms delivered identical bytes.

## Smoke history

The first dirty smoke was correctly rejected because its verbose recovery hint left only 19.65%
source reduction. Compacting the wording without removing the exact recovery command produced a
23.84% passing smoke. The full cohort still failed the provider-effect gate; the gate was never
relaxed.

## Privacy

Committed reports contain allowlisted aggregates only. They contain no command, arguments, paths,
status output, prompts, agent messages, provider bodies, session/source identifiers, or recovery
tokens.
