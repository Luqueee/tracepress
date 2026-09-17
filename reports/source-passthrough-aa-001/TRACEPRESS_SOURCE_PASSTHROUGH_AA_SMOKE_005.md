# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001 — Smoke 005

Status: **infrastructure passed; cohort required**. The clean-worktree,
single-threaded workload yielded two provider requests in both arms, one hook
rewrite and one source execution in Treatment, and no errors or timeout.

Input usage still differed between the arms even though no reducer was active.
This smoke neither attributes that difference to savings nor accepts the
passthrough A/A. The isolated N=10 cohort is the next decision point.
