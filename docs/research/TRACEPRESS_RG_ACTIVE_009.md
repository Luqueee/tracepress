# Tracepress RG Active 009

## Decision

**ACCEPT `rg_v1_active` for the measured parseable workload behind explicit opt-in.**

The default remains passthrough. This result does not authorize lossy search compression, unsafe
shell consumers, automatic activation, or a second command family.

## Parseable active cohort

Control and Treatment each ran ten sessions on the same pinned public commit. Treatment forwarded
the grouped candidate plus its exact-output recovery hint.

| Measurement | Control | Treatment |
|---|---:|---:|
| Task success | 10 / 10 | 10 / 10 |
| Raw source output | 288,780 bytes | 288,780 bytes |
| Agent-visible output | 288,780 bytes | 200,270 bytes |
| Source reduction | 0% | 30.65% |
| Provider input total | 521,550 | 521,895 |
| Provider cached input | 386,048 | 418,816 |
| Provider uncached input | 135,502 | 103,079 |
| Provider requests | 20 | 20 |
| Tool calls | 10 | 10 |
| Command retries | 0 | 0 |
| Recovery requests | 0 | 0 |

Aggregate uncached input fell by 32,423 tokens, or 23.93%. The paired Treatment-minus-Control
uncached median was -8.5 tokens; five pairs were negative and five positive. Total input was nearly
flat (+345) because cached input increased while uncached input decreased. Output rose from 1,406 to
1,545 tokens and reasoning from 470 to 488; these tradeoffs remain visible and are not relabeled as
savings.

All ten candidates passed never-worse after including 1,550 total recovery-hint bytes. They grouped
4,150 match lines. Reducer latency was 5,048 microseconds total with a 498.5 microsecond median.

## Ambiguous-output safety cohort

The independent safety cohort also ran ten sessions per arm. Every Treatment execution evaluated
the reducer, rejected the ambiguous stream, and emitted the exact 228,262 raw bytes. Across
Treatment there were ten fail-opens, zero forwarding mutations, zero recoveries, zero retries, ten
tool calls, twenty provider requests, and 10/10 objective task success.

Provider deltas in this cohort are A/A noise because agent-visible bytes were identical. They are
retained in the artifact but are not evidence of reducer savings.

## Privacy and interpretation

The committed reports contain allowlisted aggregate measurements only. They contain no command,
arguments, paths, matches, prompts, agent messages, provider bodies, session/source identifiers, or
recovery tokens.

The accepted claim is deliberately narrow: source-side lossless grouping caused a material byte
reduction and improved aggregate uncached provider input without degrading the measured trajectory.
The evidence remains workload- and model-scoped.
