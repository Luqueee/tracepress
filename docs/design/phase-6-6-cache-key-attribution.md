# Phase 6.6 cache-key attribution

Phase 6.6 tests whether the Phase 6.5 cache split follows effective `hooks` state, the mere presence
of a command-line feature override, or an observable request-prefix difference. It remains a
metadata-only Shadow study. It neither reads cache-key values nor changes a forwarded request.

## Controlled arms

```text
hooks_implicit_enabled
    unchanged user configuration; hooks enabled by the loaded configuration

hooks_explicit_enabled
    unchanged user configuration plus --enable hooks

hooks_explicit_disabled
    unchanged user configuration plus --disable hooks
```

The first two arms have the same effective feature state but differ in override presence. The last
two have the same explicit-override mechanism but opposite feature state. This makes the two
candidate explanations independently observable without inspecting private configuration content.

All arms use the pinned public repository and model, ephemeral sessions, a read-only sandbox,
`approval=never`, and zero Tracepress forwarding mutations.

## Crossover

The fixed six-round Williams schedule from Phase 6.5 is retained. Each arm occupies every position
twice, every directed transition between different arms occurs twice, and each arm occupies every
position once within each three-round task block.

## Observable-prefix gate

For every matched round, attribution requires exact equality of:

- the stable developer-text fingerprint map;
- allowlisted initial-request metadata, including request size and tool/input counts;
- initial context flags; and
- the ordered initial block manifest: kind, role, origin, semantic path, byte length, exact
  fingerprint, token estimate, detected kind, and allowlisted tool identity.

Fingerprints participate only in transient equality checks and never enter the report. Failure of
any equality blocks cache attribution and reports an observable prefix difference instead.

## Attribution rules

Symmetric relative uncached-input distance is used throughout:

```text
implicit enabled vs explicit enabled   <= 10%  (equivalent)
implicit enabled vs disabled           >= 20%  (separated)
explicit enabled vs disabled           >= 20%  (separated)
```

When the observable-prefix gate passes, equivalent enabled arms plus a separated disabled arm are
classified as `effective_hooks_state_associated`. A divergent enabled pair is classified as
`explicit_override_or_uncontrolled_variance`. Failure to separate the disabled arm records that the
Phase 6.5 divergence did not reproduce.

These classifications bound the responsible configuration dimension. They do not reveal or prove
the provider's cache-key construction, and they do not authorize an instruction policy.

## Privacy

The persisted report is an aggregate allowlist. It contains safe arm names, schedule counts,
request sizes, block/token counts, aggregate provider usage, equality decisions, thresholds, and
the final classification. It excludes prompts, commands, paths, responses, instruction text,
fingerprints, cache-key values, configuration contents, authentication material, and provider or
session identifiers. Per-run SQLite databases remain temporary and are deleted after aggregation.
