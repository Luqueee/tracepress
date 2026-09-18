# Phase 6.5 provider-cache crossover

Phase 6.5 tests whether provider cache measurements become reliable when configuration arms are
fully balanced across execution position and first-order carryover. It remains Shadow research: no
ordinary Tracepress request or instruction policy is modified.

## Arms

```text
user_config_control
    unchanged user configuration

no_hooks_null
    hooks disabled; Phase 6.4 observed an identical stable instruction map

no_extensions_treatment
    plugins + memories + hooks disabled
```

All arms use the same pinned public repository, task per round, model, ephemeral mode, read-only
sandbox, `approval=never`, and zero forwarding mutations. Safe round and position ordinals are
recorded in each workload artifact and verified before analysis.

## Williams crossover

The fixed six-round schedule is:

```text
task A: control   → null      → treatment
task A: null      → treatment → control
task A: treatment → control   → null
task B: control   → treatment → null
task B: treatment → null      → control
task B: null      → control   → treatment
```

This guarantees:

- every arm occupies every position exactly twice;
- every directed transition between different arms appears exactly twice;
- for each task, every arm occupies every position exactly once;
- the schedule is deterministic and declared before provider measurements are read.

## Structural gate

Only complete, correlated `text + developer + human_authored` leaves with exact ephemeral
fingerprints and token estimates participate. The control and `no_hooks` null arm must have
identical stable maps across all six rounds. Shared identities must retain the same token weight.

The treatment's removed and added stable classes are reported separately. Fingerprints never leave
process memory.

## Provider reliability gates

The null control must satisfy all three predeclared conditions:

```text
aggregate total-input relative delta       <= 2%
aggregate uncached-input relative delta    <= 10%
maximum per-position uncached delta        <= 20%
```

Relative null deltas use symmetric distance around the pair mean. Treatment changes use control as
the denominator and are reported only after the null controls are evaluated.

A passing provider gate means the crossover is eligible for a later quality-gated experiment. It
does not establish task quality, generalize beyond the public Search workload, or authorize an
instruction policy.

## Privacy boundary

The persisted report contains only aggregate usage, token estimates, safe arm names, schedule
ordinals, thresholds, and decisions. It excludes prompts, commands, paths, responses, instruction
text, fingerprints, config contents, authentication material, and session/request identifiers.
Per-run SQLite databases remain temporary and are deleted after aggregation.
