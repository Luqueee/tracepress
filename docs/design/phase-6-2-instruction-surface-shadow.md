# Phase 6.2 instruction surface Shadow

Phase 6.2 measures repeated developer instruction exposure in captured provider requests. It does
not remove, rewrite, reorder, summarize, or select instructions.

## Question

The Phase 6.1 public cohort exposed no explicit tool definitions, while a follow-on aggregate
diagnostic showed that developer-role text dominated the locally estimated explicit context. Before
designing any context policy, Tracepress must determine whether that text is stable across requests
and sessions and whether provider cache behavior already absorbs most of the repeated exposure.

## Measurement boundary

Only leaf blocks satisfying all of the following contribute to instruction totals:

```text
kind   = text
role   = developer
origin = human_authored
```

Parent message bytes are excluded to avoid double counting. Unknown developer-role blocks are
reported separately by count and bytes; they receive no invented token estimate.

The aggregate reports:

- complete latest-snapshot coverage;
- developer text block count, bytes, and locally estimated tokens;
- estimated tokens whose exact fingerprint recurs across distinct provider requests;
- estimated tokens whose exact fingerprint recurs across distinct sessions;
- repeated exposure beyond one copy per cross-request recurring fingerprint;
- unknown developer block count and bytes; and
- provider-reported input, cached, uncached, output, and reasoning totals for the same latest-request
  cohort.

Fingerprints exist only inside the read-only aggregate query. They, semantic paths, session IDs,
request IDs, instruction text, prompts, commands, paths, and responses never enter the report.

## Integrity gates

Instruction totals are unavailable unless every latest snapshot is complete, explicitly complete,
and correlated, and every provider request in the observed sessions has a latest snapshot. A
snapshot may remain `explicit_only` or `provider_managed_partial`: this pilot measures only leaf
blocks physically present in the explicit request and does not infer referenced history. Token and
repetition metrics also require every included developer text leaf to have both an estimate and an
exact fingerprint. Missing evidence is `null`, not zero.

Repeated exposure excludes duplicates confined to one request. Cross-session recurrence requires
the same fingerprint in at least two distinct sessions.

## Decision boundary

This phase can establish an observable repeated instruction surface. It cannot establish that a
block is unnecessary, user-controlled, safe to remove, or causally responsible for cached or
uncached provider input. Positive recurrence therefore advances only to source attribution.

No active instruction policy is eligible until a later phase identifies the source/configuration
layer of each candidate class, defines an objective task-quality evaluator, and passes an isolated
session-level control/treatment experiment.
