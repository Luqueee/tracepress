# Tracepress Cache-Key Equality 020

## Decision

**PASS ephemeral cache-key equality attribution. Independent Codex sessions use distinct key
classes; keep cross-session cache causality blocked.**

Phase 6.7 ran all three Phase 6.6 arms through one loopback equality observer and one Tracepress
proxy. All 18 ephemeral Codex sessions succeeded. The observer decoded request bodies for analysis
only, assigned ordinal equality classes in memory, and forwarded the original compressed bytes
unchanged.

## Equality result

| Measurement | Result |
|---|---:|
| Child sessions | 18/18 successful |
| Provider requests | 36/36 correlated |
| Requests with a string cache key | 36/36 |
| Unique equality classes | 18 |
| Requests per session | 2 in all 18 sessions |
| Same class within each session | 18/18 |
| Cross-arm matched rounds sharing a class | 0/18 pair-rounds |

Each child session used one cache-key class for both of its requests. Every subsequent child session,
including sessions in the same task block and sessions with the same effective `hooks` state, used
a different class. The classes were assigned in first-seen order and have no stable meaning outside
this process.

This directly identifies the hidden difference left unresolved by Phases 6.5 and 6.6. Those phases
compared independent sessions, so their control and treatment requests did not share cache-key
identity even when their stable developer instructions matched.

## Provider observation

| Arm | Input | Cached | Uncached |
|---|---:|---:|---:|
| Implicitly enabled | 159,373 | 119,808 | 39,565 |
| Explicitly enabled | 158,971 | 144,384 | 14,587 |
| Explicitly disabled | 159,125 | 135,168 | 23,957 |

Total input remained close while uncached allocation differed. The symmetric uncached distances
were 92.25% for implicit versus explicit enabled, 49.14% for implicit enabled versus disabled, and
48.62% for explicit enabled versus disabled. None is a feature effect because every comparison also
crosses a cache-key class boundary.

## Integrity and privacy

- The observer forwarded the original compressed request bodies byte for byte.
- Zstandard decoding occurred only in memory and only for equality observation.
- Key values were held only in the child process and destroyed at exit.
- No key value, key length, reusable hash, per-request class, prompt, response, command,
  authentication material, request identifier, or session identifier appears in the report.
- The published artifact contains only counts and aggregate provider usage.

## Consequence

The Phase 6.5 cache split is now bounded to a session-scoped cache-key confound plus the dynamic
prefix differences identified in Phase 6.6. It cannot support a cached/uncached treatment claim or
an active instruction policy.

A further causal cache experiment would require an explicitly active, controlled cache-key
normalization intervention so matched arms share one cohort-scoped key. That would mutate provider
requests and needs a separate safety, privacy, and fail-open design gate. Until such a phase is
approved, cache-split research stops here; total-input measurements remain the only provider-input
comparison supported by the current Shadow surface.
