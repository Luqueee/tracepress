# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001 — Results 004

Status: **rejected as a causal passthrough A/A**. No reducer was enabled.

All ten Treatment sessions recorded one hook rewrite and one source execution;
all twenty sessions completed with no provider errors or timeouts. Each arm used
an isolated pinned worktree, a private ephemeral Cargo target directory, and
single-threaded tests. Treatment emitted 20,674 source bytes per execution.

Despite the absence of a reducer, Treatment recorded 308,562 provider input
tokens (101,714 uncached) while Control recorded 433,889 (158,689 uncached).
Provider requests were 20 and 21 respectively. The task-quality and retry
signals do not show a regression, but the large input delta has no established
causal explanation under passthrough. It must not be called source reduction
or provider savings.

Phase 5.0 is therefore blocked before Shadow Source Mode: the Codex rewrite
integration must expose a byte-faithful, agent-visible control comparison (or
another source-side mechanism) before `cargo_test_v1` can be evaluated.
