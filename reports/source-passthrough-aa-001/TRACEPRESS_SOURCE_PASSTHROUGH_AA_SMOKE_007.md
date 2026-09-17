# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001 — Smoke 007

Status: **infrastructure passed; full cohort required**. Both arms succeeded
with two provider requests and no timeout. Treatment recorded exactly one hook
rewrite, one source execution, and 20,674 emitted bytes. Each arm had an
independent worktree and `CARGO_TARGET_DIR`.

Provider usage remains an observational difference while passthrough is active;
this smoke does not label it a source-reduction effect. The N=10 cohort is
needed to decide whether the wrapper can meet the behavior-equivalence gate.
