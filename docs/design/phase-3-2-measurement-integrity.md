# Phase 3.2 — Measurement Integrity & Deferred Analysis

Phase 3.2 separates durable request outcomes from operational analysis-drop events and defers
context analysis when forwarding is still active. It does not change the Phase 3 parser,
detectors, fingerprints, token estimator, or context schema.

## Gate A — request analysis ledger

`scripts/analyze_baseline.py` builds one metadata-only ledger row per distinct
`provider_request_id`. For eligible requests, the exclusive outcome is derived in this order:

```text
Complete snapshot  -> Complete
terminal snapshot -> Partial
no terminal snapshot -> Dropped
ineligible request -> Ineligible
```

An auxiliary `context.analysis.dropped` event is counted separately. It does not override a
durable `Complete` snapshot. A recovered snapshot (`recovered_at_us != NULL`) is a durable
`Partial`, not a drop. Compaction transport observations are currently `Ineligible` for context
analysis because Phase 3.1.2 does not yet create a context snapshot for them; they remain covered
by separate compaction transport reports. Aggregate events that cannot identify a request are
reported as unmatched and invalidate measurement integrity rather than being assigned by
inference.

The analyzer fails with a non-zero status and the convergence checker returns
`NEED_MORE_SESSIONS` whenever the ledger partition is contradictory or measurement integrity is
failed.

The existing cohorts were recalculated without new sessions:

| Cohort | Eligible | Complete | Partial | Dropped | Request coverage | Integrity |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Pilot n20 | 39 | 39 | 0 | 0 | 100.00% | passed |
| Concurrent n24 | 121 | 116 | 0 | 5 | 95.87% | failed: 15 unmatched aggregate drop events |
| Serial n3 | 10 | 10 | 0 | 0 | 100.00% | failed: 1 unmatched aggregate drop event |

The concurrent cohort's five genuine missing requests are retained in the report with their
provider request IDs and sizes. The fifteen aggregate events account for 62 auxiliary work units,
but do not identify those requests; they are therefore not used to manufacture request outcomes.

## Gate B — bounded deferred analysis

The concurrent evidence showed missingness concentrated in late, larger requests, so the runtime
now defers analysis instead of dropping it merely because another forward is active:

- admission reserves only the raw `WireBody`, bounded by 32 items and 64 MiB;
- a single FIFO worker per proxy/session processes admitted requests in monotonic `ForwardId`
  order;
- the worker waits for forwarding to become idle before CPU-bound decoding and analysis;
- provider evidence settles its correlation identity immediately and retains only a compact
  metadata receipt until the context outcome arrives;
- analyzed outcomes are admitted through a bounded semaphore before entering the recorder, so
  the context result path cannot retain an unbounded number of heavy analysis objects;
- `DeferredBacklogCapacity` is the explicit hard-overflow reason;
- queue items, bytes, high-water marks, deferred/processed counts, capacity drops, and wait time
  are reported without payloads;
- shutdown keeps the background barrier alive so queued analysis drains before recorder closure.

The provider recorder remains non-blocking with respect to forwarding and fails auxiliary event
admission fast when its bounded channel is full. The deferred queue retains compressed wire bytes.
Zstd decoding still happens only in the analysis path. Standalone proxy shutdown waits on the
background barrier up to the configured drain timeout.

Because Gate B changes runtime admission/execution, it increments
`measurement_instrument_version` to `2`. The reports in this document were generated from the
pre-Gate-B cohorts with version `1`; they are validation evidence only. A convergent baseline
must start as `baseline-002` with the new version and must not mix those sessions into its
convergence transitions.

## Validation

The workspace validation for this change is:

```text
cargo fmt --all -- --check       passed
cargo clippy --workspace ...     passed with -D warnings
cargo test --workspace           489 passed, 1 ignored
baseline analysis contracts    13 passed
```

No new measurement sessions are started by this phase. The adaptive baseline remains paused until
the new runtime instrument is frozen and a fresh cohort is collected.
