# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001 — Results 002

Status: **rejected as an A/A result**. Passthrough only; no reducer was enabled.

All ten Passthrough sessions produced exactly one Codex hook rewrite and one
source execution, with no timeouts or provider errors. That proves the hook and
proxy instrumentation were observed in the full cohort.

The result cannot establish passthrough equivalence: each Control/Passthrough
pair reused the same checkout, so Control could warm Cargo artifacts that
changed the later command output. Provider-input and request-count deltas are
therefore confounded and must not be interpreted as a proxy effect.

The runner now creates a fresh detached worktree for every arm and alternates
arm order. Results 002 remain as negative methodology evidence; the isolated
rerun is required before Shadow Source Mode.
