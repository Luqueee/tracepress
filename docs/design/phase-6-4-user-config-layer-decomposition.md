# Phase 6.4 user-config layer decomposition

Phase 6.4 decomposes the stable instruction surface associated with loading Codex user
configuration. It remains a controlled public Shadow experiment: no ordinary Tracepress request,
instruction, tool definition, or provider payload is changed.

## Controlled arms

Seven arms run the same pinned public repository, model, read-only sandbox, task for each round, and
ephemeral session behavior:

```text
user_config_a                 unchanged user config, A/A control
user_config_b                 unchanged user config, A/A control
no_plugins                   user config loaded, plugins feature disabled
no_memories                  user config loaded, memories feature disabled
no_hooks                     user config loaded, hooks feature disabled
no_extensions                plugins + memories + hooks disabled
ignore_user_config           Codex --ignore-user-config
```

Feature names are a fixed allowlist. Arbitrary config paths or values cannot enter the runner. An
ablation measures the runtime composition effect of a feature flag; it does not identify literal
text in `config.toml` and is not a proposed user-facing policy.

## Interleaved cache design

Each round uses one bounded public Search task. Every arm runs once per round, the A/A controls are
adjacent with alternating order, and the remaining arms rotate. Each workload artifact records a
safe round ordinal and schedule position. The aggregate gate verifies both before claiming the
interleaved schedule.

Provider usage is a separate observational surface. The predeclared cache gate is:

```text
abs(A_uncached - B_uncached) / mean(A_uncached, B_uncached) <= 10%
```

The `no_hooks` arm is also a structural null control. When its stable map matches the unchanged
baseline, its uncached-input delta must pass the same 10% threshold. Both controls must pass before
cross-arm provider comparisons are considered reliable. Failure blocks provider-effect and savings
conclusions but does not invalidate an exact structural decomposition.

## Structural contract

Only complete, correlated `text + developer + human_authored` leaves with exact fingerprints and
token estimates participate. Identities remain ephemeral.

- A block is stable only when it has the same identity, multiplicity, and weight in every request
  and every round of an arm.
- The two unchanged A/A arms must have identical stable maps.
- Cross-arm common identities must retain the same weight.
- Removed and added classes are reported separately; net differences are never mislabeled as
  literal source text.
- Independent removals are compared with the combined arm so interactions remain visible.

## Privacy and decision boundary

Reports contain only aggregate counts, token estimates, usage totals, safe arm names, and safe
ordinals. They exclude instruction text, prompts, commands, paths, responses, fingerprints,
configuration contents, authentication material, and session/request identifiers. Databases exist
only in a temporary directory and are deleted after aggregation.

A passing structural gate permits a decomposition report only. It does not enable an instruction
policy. Any active experiment still requires an objective quality evaluator and a provider-effect
design whose A/A cache noise passes before treatment is interpreted.
