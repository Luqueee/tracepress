# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001 — Smoke 004

Status: **infrastructure passed; equivalence not established**. Each arm used a
clean detached worktree. Treatment recorded one hook rewrite and one source
execution with no provider errors or timeout.

The agent nevertheless issued four provider requests in Treatment versus two in
Control. This smoke is not evidence of a passthrough effect or of a regression;
it demonstrates that the A/A must use a deterministic test ordering and a
larger isolated cohort before passing the behavior-equivalence gate.
