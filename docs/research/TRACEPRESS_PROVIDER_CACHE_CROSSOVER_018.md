# Tracepress Provider Cache Crossover 018

## Decision

**PASS total-input crossover control. FAIL cache-split reliability. Keep active instruction policy
blocked.**

The certified cohort ran three controlled arms through a two-task, six-round Williams crossover.
All 18 ephemeral public sessions succeeded and produced 36/36 complete provider-request snapshots.
Every arm occupied every position once for each task, and every directed arm transition appeared
twice.

## Structural result

The unchanged control and `no_hooks` null arm had exactly the same stable instruction map at 21,445
estimated tokens/request. The `no_extensions` treatment remained at 7,301 tokens/request, with
15,684 stable tokens removed and a distinct 1,540-token class added.

This independently reproduces the Phase 6.4 structural result under a different schedule.

## Provider control result

| Arm | Input | Cached | Uncached |
|---|---:|---:|---:|
| Control | 280,613 | 265,216 | 15,397 |
| Structurally identical `no_hooks` null | 280,458 | 218,112 | 62,346 |
| `no_extensions` treatment | 136,025 | 113,664 | 22,361 |

The null control passed total-input reliability: its symmetric relative delta was **0.06%**, below
the predeclared 2% gate. It decisively failed cache-split reliability:

| Cache control | Observed | Gate |
|---|---:|---:|
| Aggregate uncached relative delta | 120.78% | ≤10% |
| Position 0 uncached delta | 127.40% | ≤20% |
| Position 1 uncached delta | 116.14% | ≤20% |
| Position 2 uncached delta | 119.02% | ≤20% |

Because the failure is large and present in every fully task-matched position, it is not explained
by arm order or task assignment. It is consistent with an unobserved cache-key or cache-eligibility
difference caused by feature configuration, but this experiment does not identify that mechanism.

## Treatment observation

Relative to control, the treatment produced:

- total input: **-51.53%**;
- cached input: **-57.14%**;
- uncached input: **+45.23%**;
- output: **+9.53%**;
- reasoning: **+14.31%**.

Only the total-input comparison passed its matching null control. Cached and uncached changes are
not interpretable as treatment effects. Even the total-input reduction does not pass an active
policy gate because this phase did not include an objective task-quality evaluator, retry metric,
or broader task set.

## Integrity and privacy

- Sandbox, approval policy, model, repository pin, ephemeral mode, feature profile, task block,
  schedule position, and zero forwarding mutations were verified for every run.
- A development cohort that changed tasks every round was discarded before publication because its
  per-position cells confounded task and position.
- The certified cohort used the same task within each three-row Latin block and the reverse design
  for the second task.
- Fingerprints remained ephemeral. No instruction text, prompt, command, path, response, config
  content, authentication material, session ID, or request ID was persisted.

## Consequence

Phase 6.5 shows that total provider input can be compared under the balanced crossover, but provider
cache accounting cannot. The next eligible work is a metadata-only cache-key attribution Shadow
study comparing the structurally identical control and `no_hooks` requests before considering any
quality-gated treatment pilot.
