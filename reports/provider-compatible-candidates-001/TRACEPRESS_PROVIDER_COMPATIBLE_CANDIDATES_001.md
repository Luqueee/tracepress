# Tracepress Provider-Compatible Candidate Discovery 001

Status: **blocked pending Naturalistic Shadow Pilot 003 (N=10)**

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

## Evidence and gate

The available characterization artifact is a one-session probe (`TRACEPRESS_PROVIDER_COMPATIBLE_CHARACTERIZATION_PROBE_001`), not the required naturalistic N=10 cohort. It observed one ToolResult JSON block and selected no raw content. Therefore no human-readable candidate is promoted to active A/B and Phase 4.3 remains blocked.

Recovery/determinism/unit, persistence, read-only API, and workspace tests pass. The remaining empirical action is to run Shadow Pilot 003 against ten real sessions and compare addressable token share plus effective reduction; no provider-token, cache, cost, or quality claim is made here.
