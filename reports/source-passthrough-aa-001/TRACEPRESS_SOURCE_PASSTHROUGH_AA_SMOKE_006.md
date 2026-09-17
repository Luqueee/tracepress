# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001 — Smoke 006

Status: **diagnostic only; rejected for equivalence**. Treatment emitted 20,620
bytes through the passthrough proxy, while Control's provider input was much
larger. This exposed a remaining workload confound: Cargo could inherit a
shared `CARGO_TARGET_DIR`, so the two arms did not both perform a clean build.

The runner now sets an ephemeral `CARGO_TARGET_DIR` inside each arm state.
Smoke 006 must not be used as performance or token evidence.
