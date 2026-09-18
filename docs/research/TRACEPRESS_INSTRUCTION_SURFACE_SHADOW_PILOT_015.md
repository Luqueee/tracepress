# Tracepress Instruction Surface Shadow Pilot 015

## Decision

**PASS the repeated-surface characterization. Require source attribution before policy design.**

The controlled public cohort completed 10 of 10 sessions successfully under `codex-cli 0.155.0`
with model `gpt-5.6-luna`. Tracepress analyzed all 21 observed provider requests without modifying
instructions or forwarding.

## Evidence

| Metric | Result |
|---|---:|
| Sessions complete and successful | 10/10 |
| Provider requests complete | 21/21 |
| Developer text leaf blocks | 147 |
| Developer text bytes | 1,240,344 |
| Developer estimated tokens | 451,710 |
| Cross-request recurring estimated tokens | 451,710 |
| Cross-session recurring estimated tokens | 451,710 |
| Repeat exposure beyond one copy | 430,200 estimated tokens |
| Repeat exposure share | 95.24% |
| Unknown developer blocks | 21 |
| Unknown developer bytes | 653,457 |
| Provider input tokens | 505,119 |
| Provider cached input tokens | 489,216 |
| Provider uncached input tokens | 15,903 |
| Provider cache ratio | 96.85% |

The developer-token values are local estimates over non-overlapping text leaves. Provider usage is
provider-reported. They are adjacent measurements with different provenance and must not be
subtracted from one another or presented as causal savings.

An initial N=10 artifact was discarded after a test-first coverage canary showed that counting only
existing snapshots could miss a provider request without a snapshot. The final cohort above was
rerun after the gate required equality between requests in observed sessions, latest snapshots, and
complete snapshots. Its 21 requests all passed that stronger contract.

## Interpretation

The explicit developer text is highly stable across both requests and sessions: every estimated
developer text token belonged to a cross-session recurring fingerprint, and 95.24% of exposure was
beyond the first observed copy. At the same time, provider-reported cache usage was also high.

This establishes a large repeated surface, but not a large uncached-cost opportunity. It also does
not establish that any block is unnecessary or user-controlled. The correct next step is structural
source attribution, not deletion or summarization.

## Integrity and privacy

- Parent message bytes were excluded to avoid overlapping with their text leaves.
- Duplicates confined to one request cannot become cross-request repeat exposure.
- A provider request in an observed session without a latest snapshot makes instruction totals
  unavailable.
- Explicitly complete snapshots remain eligible even when logical history is provider-managed,
  because this pilot measures only blocks physically present in the request.
- Unknown developer blocks remain byte-only and receive no invented token estimate.
- Fingerprints were used ephemerally inside the read-only aggregation query and were not persisted
  in the report.
- No instruction text, prompt, command, path, response, semantic path, session ID, or request ID is
  present in the artifacts.

## Next gate

Phase 6.3 may attribute stable block classes to controlled sources such as base Codex instructions,
workspace instructions, plugins, or generated runtime context. It must use isolated configuration
and structural deltas only. Active instruction mutation remains blocked until attribution, objective
quality evaluation, and a session-level A/A control exist.
