# Tracepress Cache-Key Attribution 019

## Decision

**PASS the observable-prefix guard. REJECT cache-key attribution from independent Codex sessions.**

The certified Phase 6.6 cohort ran 18 successful ephemeral sessions through the same two-task,
six-round Williams crossover used in Phase 6.5. It compared implicit `hooks` enablement, an explicit
`--enable hooks` override, and an explicit `--disable hooks` override.

All three arms retained the same stable developer-text map. They did not retain the same complete
initial request prefix, so provider cache differences cannot be assigned to feature state or
override presence.

## Observable-prefix result

Every initial request contained 12 observed blocks, but exact equality failed in seven block
classes. Divergence occurred in composite developer and user messages and in the initial user
context preceding the controlled task. The controlled task leaf itself remained equal.

| Gate | Result |
|---|---|
| Stable developer-text maps | Exact match |
| Initial request metadata | Request byte length differed in 4/6 matched rounds |
| Complete initial block manifest | Failed in 6/6 rounds |
| Divergent block classes | 7 |
| Cells with fingerprint differences | 38 |
| Cells with byte-length differences | 9 |
| Cells with token-estimate differences | 2 |

The report preserves only semantic paths and mismatch field names. Exact fingerprints were compared
transiently and were not serialized.

## Provider result

| Arm | Input | Cached | Uncached |
|---|---:|---:|---:|
| Implicitly enabled | 135,199 | 90,368 | 44,831 |
| Explicitly enabled | 135,111 | 113,664 | 21,447 |
| Explicitly disabled | 134,571 | 110,592 | 23,979 |

The implicit/explicit-enabled uncached distance was **70.56%**. The explicit-enabled/disabled
distance was **11.15%**: above the predeclared 10% equivalence bound, but below the 20% separation
bound. The implicit-enabled/disabled distance was **60.61%**.

The Phase 6.5 control/null separation of 120.78% therefore did not reproduce as an effective-hooks
contrast. A complete development cohort collected before the final mismatch summary reached the
same prefix-gate decision while producing a materially different allocation of cached tokens. It
was discarded and not used for the certified measurements. Together, these results show that
provider cache allocation varied across otherwise controlled independent sessions while dynamic
prefix material was still present.

## Consequence

Phase 6.6 explains why the Phase 6.5 null was invalid more narrowly than an unspecified cache-key
effect: equality of stable developer text did not imply equality of the complete cache-relevant
prefix. The current independent-session surface cannot distinguish configuration causality from
dynamic per-session prefix material and provider-managed cache state.

No cached or uncached treatment claim is permitted. Total-input comparisons remain separately
observable, but this phase did not evaluate task quality and authorizes no active instruction
policy.

The next eligible work is an ephemeral equality observer that compares cache-key presence and
within-cohort equality in one process while persisting neither values nor reusable hashes. It must
also identify which dynamic prefix classes precede the controlled task before another provider
cache experiment is considered.
