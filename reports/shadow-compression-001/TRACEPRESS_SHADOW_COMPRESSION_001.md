# Tracepress Shadow Compression 001

Status: **Shadow Pilot 001 completed with degradation; remaining empirical gates pending**.

Phase 4.0 is frozen at `93ffe0c9c32a0f9a78027c6159e70236b2e6a598` under the
`phase-4.0-complete` tag. Phase 4.1 remains additive relative to that point.

## What is operational

The shadow path now targets only `ToolGenerated + ToolResult + Json` and the explicitly safe
`ToolGenerated + ToolResult + PlainText` subset. `UnknownTransformPolicy` is `Never`. The no-op,
JSON minification, homogeneous tabular JSON, exact repeated-subtree, repeated-line, and bounded
repeated-run implementations are deterministic, resource-bounded, and recovery-checked. Candidate
and recovery content remains ephemeral; SQLite receives metadata only through `tracepressd`.

The worker has an independent bounded queue and byte budget. Admission happens after durable Phase
3 context finalization, uses non-blocking `try_send`, and never feeds candidate bytes back into the
forwarding path.

## Controlled end-to-end validation

Experiment `shadow-e2e-final` sent one 327-byte request through the real proxy into a deterministic
local upstream. The upstream observed the same 327 bytes. The persisted experiment completed with:

| Metric | Result |
|---|---:|
| Forwarding mutations | 0 |
| Shadow drops | 0 |
| Recovery failures | 0 |
| Determinism failures | 0 |

The single homogeneous JSON ToolResult produced the following local representation evidence:

| Candidate | Status | Input bytes | Output bytes | Byte reduction | Recovery |
|---|---|---:|---:|---:|---:|
| `json.noop` | NoImprovement | 190 | 190 | 0 | verified |
| `json.minify` | Applicable | 190 | 75 | 115 | verified |
| `json.tabular` | Applicable | 190 | 63 | 127 | verified |
| `json.repeated_subtree` | NotApplicable | 190 | — | — | unavailable |

These values are a controlled plumbing check, not a workload result. They must not be interpreted
as provider token savings, cost savings, cache preservation, or evidence that model quality is
preserved.

## Shadow Pilot 001 — N10 naturalistic

Ten valid read-only Codex sessions using `gpt-5.6-luna` at `xhigh` produced 75 provider requests
and 1,666 persisted candidate evaluations. Two additional collection attempts are retained but
explicitly excluded: one interrupted review session and one CLI argument failure with zero provider
requests. The auditable scope and workload labels are in `experiment_manifest.json`; the generated
results are in `shadow_pilot_n10.json` and `shadow_pilot_n10.md`.

| Metric | Result |
|---|---:|
| Forwarding mutations | 0 |
| Shadow drops | 15 |
| Recovery failures | 0 |
| Determinism failures | 0 |
| Unknown transformed | 0 |

`json.minify` produced `NoImprovement` for all 424 evaluated blocks. `json.repeated_subtree` was
not applicable to all 409 evaluated blocks. `json.tabular` was applicable to 179 of 409 evaluations,
but its effective byte reduction was only 7,299 bytes (0.12%) and its locally estimated reduction
was 3,864 tokens (0.18%). Reduction over unique fingerprints was only 332 bytes. All applicable
tabular results came from the `repo_exploration` workload, so the signal is workload-concentrated
and not material enough to select an active candidate.

The first, unusually broad 30-request session saturated the shadow path. Those 15 historical drops
were collected before reason-specific accounting existed, so they cannot be attributed retroactively
to queue-full, byte-budget, work-budget, worker closure, or persistence failure. Migration v10 now
records each reason for subsequent experiments; bounded sessions after the saturation added no drops.
The pilot is therefore complete but degraded, and does not pass the Shadow Gate.

## Security review

Codex Security scan `505e56ef-05c4-48f0-ac68-43fd92a5cfdf` found one low-severity resource-accounting
issue in repeated-subtree factoring. The implementation now charges retained subtree encodings
before insertion and rejects recovery inputs/allocation lengths before parsing or reserving. The
focused regression tests, fuzz-target compilation, full workspace suite, and final controlled E2E
run pass after the remediation. No raw candidate content is persisted or logged.

The N10 code-review workload found two additional bounded-input issues after collection: an input
could exceed `max_shadow_memory_bytes` before JSON parsing, and the tenth byte of a recovery varint
could contain more than the single valid high bit. The same tenth-byte guard is now applied to the
plain-text recovery decoder. All three cases are rejected before unsafe allocation/decoding and
have focused regression tests. A post-hardening confirmation smoke against a local
deterministic upstream passed with byte-exact forwarding, persisted candidates, zero recovery
failures, and reason-specific drop counters summing to the aggregate total.

The post-hardening A/A smoke ran the same ToolResult request with Shadow OFF and Shadow ON (no
request mutation). Provider request bytes, input/cache usage, output, and reasoning observations
were equal in both arms; both arms forwarded the exact request and the ON arm recorded zero drops,
recovery failures, and determinism failures. This is an infrastructure smoke, not a statistical
performance or quality claim.

## Compaction calibration

A separate controlled cohort of five isolated sessions exercised `normal turn → manual compact →
post-compaction turn` against a deterministic local upstream. It produced 15 provider requests,
five `compaction_v2` requests, 15 complete context snapshots, and ToolResult shadow candidates on
the surrounding turns. Forwarding mutations and persisted candidate content remained at zero. The
cohort validates lifecycle/block-survival plumbing only; it is not naturalistic workload, cache, or
quality evidence. See `compaction_calibration.json`.

## Context-only A/A reference

The existing Phase 3 harness also ran one `small_json` sample with context analysis OFF and ON.
Both arms had zero request errors; ON measured 9,768.7 µs dispatch / 240.8 µs TTFB / 440 KiB idle
RSS versus OFF at 9,824.1 µs / 458.3 µs / 452 KiB. Peak RSS was 7,028 KiB ON versus 6,856 KiB
OFF. This is a single-sample context-analysis reference, not a Shadow Compression resource gate;
the dedicated Shadow OFF/ON resource cohort remains pending. See `phase3_aa_smoke.json`.

A dedicated Shadow Compression resource smoke then ran two measured samples (one warmup) of an
eligible `ToolResult JSON` workload. Shadow ON versus OFF measured −218.2 µs dispatch, −162.8 µs
proxy TTFB, −416.1 µs duration, −12 KiB idle RSS, and +396 KiB peak RSS, with zero request errors
and zero forwarding mutations; see `shadow_compression_resource_smoke.json`.

A broader five-sample A/A resource comparison (two warmups per arm) recorded zero request errors
and zero forwarding mutations. ON versus OFF had +38.0 µs dispatch, −31.8 µs proxy TTFB, +23.2 µs
duration, +4 KiB idle RSS, and +384 KiB peak RSS; all timing intervals crossed zero or stayed
inside the measured A/A noise envelope. The per-arm child CPU accounting recorded +13.95 ms
ON-minus-OFF total CPU (+26.28 ms user, −12.33 ms system). This includes process startup/teardown
and is observational rather than a host-wide CPU SLO; the sample is not a stable resource claim.
See `shadow_compression_resource_aa_n5_cpu.json`.

## Remaining empirical gates

- A follow-up naturalistic cohort if a candidate or workload target changes enough to plausibly
  produce material local reduction. The current `json.tabular` signal is workload-concentrated
  and only 0.12% effective byte reduction.
- Longer resource characterization if the project needs a stable CPU/RSS baseline; the current
  five-sample measurement is explicitly observational and lifecycle-inclusive.

No active candidate is selected while these gates are pending. Phase 4.2 remains blocked. After
the pilot and A/A checks, this report must be regenerated with exactly one recommended candidate or
an explicit decision that no candidate qualifies.
