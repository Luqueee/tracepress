# Tracepress Provider-Compatible Candidate Discovery 001

Status: **completed — no material provider-compatible candidate**

Phase 4.0 remains frozen at `93ffe0c9c32a0f9a78027c6159e70236b2e6a598` (`phase-4.0-complete`).
Forwarding mutations remain **0** and candidate bodies are not persisted.

## Implemented

- `json.minify` is formally retained as a provider-compatible control and rejected as an active candidate after the real provider diagnostic found no material improvement.
- Human-readable, reversible, deterministic, bounded candidates: `json.readable_table`, `json.compact_records`, `json.key_elision`, `text.readable_line_fold`, `text.readable_block_fold`, and `text.log_prefix_fold`.
- `json.tabular`/TPJ2 remains an opaque decoder-dependent shadow upper bound.
- Unknown remains `NeverTransform`.
- SQLite migration 0011 persists provider readability and metadata-only JSON/plain-text shape labels.
- Observatory Compression Lab exposes addressable estimated-token share, applicability, readability, shape, reduction, recovery, latency, and cache-risk evidence.
- `scripts/characterize_provider_shapes.py` provides a read-only, metadata-only characterization command.
- Naturalistic Shadow Pilot 003 completed 10 sessions / 31 provider requests with 31/31 analysis-complete snapshots.

## Evidence and gate

The naturalistic Pilot 003 observed no material reduction for `json.readable_table`, `json.compact_records`, or `json.key_elision` (0% addressable/effective reduction in the cohort). `json.minify` remained a no-improvement control. Recovery and determinism were 100% for evaluated candidates, Unknown transformed was 0, and forwarding mutations were 0. The run recorded 24 bounded shadow drops, so it is marked degraded for resource characterization.

The separate one-session probe (`TRACEPRESS_PROVIDER_COMPATIBLE_CHARACTERIZATION_PROBE_001`) remains a shape smoke only. No human-readable candidate is promoted to active A/B and Phase 4.3 remains blocked.

Recovery/determinism/unit, persistence, read-only API, and workspace tests pass. The deterministic/reversible opportunity is currently insufficient for active compression; any next strategy would require a separately designed lossy/quality-evaluation phase. No provider-token, cache, cost, or quality claim is made here.
