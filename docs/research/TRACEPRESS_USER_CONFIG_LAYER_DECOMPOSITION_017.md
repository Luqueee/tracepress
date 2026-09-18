# Tracepress User Config Layer Decomposition 017

## Decision

**PASS structural layer decomposition and adjacent A/A. FAIL the structural-null cache control.
Keep active instruction policy blocked.**

The certified cohort ran seven controlled arms across three interleaved rounds: 21/21 ephemeral
public sessions completed successfully and produced 42/42 complete provider-request snapshots.
The unchanged A/A arms had identical stable maps with zero tokens exclusive to either arm.

## Structural result

| Effect | Removed tokens/request | Added tokens/request | Net stable change |
|---|---:|---:|---:|
| Disable plugins | 9,973 | 1,540 | -8,433 |
| Disable memories | 5,711 | 0 | -5,711 |
| Disable hooks | 0 | 0 | 0 |
| Disable all three | 15,684 | 1,540 | -14,144 |

The unchanged user-config surface was 21,445 stable estimated tokens/request. The combined ablation
and `--ignore-user-config` both converged to 7,301. Their exact stable maps matched: neither side had
an exclusive stable class. The independently observed plugin and memory removals explain all 15,684
tokens removed by the combined arm; no combined-only interaction class was observed. A 5,761-token
stable class remained common to all seven arms.

These are structural feature effects, not removable file contents. Disabling plugins also introduced
a distinct 1,540-token stable class, so the plugin effect must be represented as removed and added
classes rather than only a net subtraction.

## Cache A/A result

| Unchanged arm | Input | Cached | Uncached |
|---|---:|---:|---:|
| `user_config_a` | 140,338 | 103,936 | 36,402 |
| `user_config_b` | 140,224 | 103,936 | 36,288 |
| Structurally identical `no_hooks` | 140,169 | 133,632 | 6,537 |

The adjacent uncached-input A/A delta was **0.31%**, below the predeclared 10% threshold. However,
the `no_hooks` arm had the exact same stable instruction map and nearly identical total input
(**0.08%** relative delta), while its uncached-input relative delta was **139.02%**. It therefore
fails the same 10% structural-null threshold. Provider cache splits for the ablation arms remain
descriptive only. This experiment does not claim that disabling any layer saves provider input,
latency, or cost.

The null-control failure demonstrates why a clean adjacent A/A is insufficient when treatment arms
occupy different cache positions, and why structural token counts cannot substitute for provider
usage.

## Integrity and privacy

- Every arm proved its exact feature profile, task round, schedule position, model, repository pin,
  read-only workload contract, ephemeral mode, and zero forwarding mutations.
- The final certified N=3 cohort was run with artifact-level schedule evidence plus explicit,
  identical sandbox and approval policy; earlier development cohorts were not used in this report.
- Fingerprints were compared only in memory and were never serialized.
- No prompt, command, path, response, instruction text, config content, authentication material,
  session ID, or request ID was persisted.
- No global Codex config or authentication file was modified or copied.

## Consequence

Phase 6.4 identifies plugins and memories as the complete stable decomposition of the observed
user-config-associated delta for this workload; hooks contribute no stable structural delta. It
does not authorize disabling either feature. The next eligible work is a fully position-balanced
provider-cache crossover baseline that can reduce or explain null-control variance before any
active instruction experiment.
