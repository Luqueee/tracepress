# TRACEPRESS_SOURCE_PASSTHROUGH_AA_001 — Results 003

Status: **rejected as a passthrough A/A**. No reducer was enabled.

The infrastructure invariant passed: all ten Treatment sessions had one Codex
hook rewrite and one source execution; neither arm had provider errors or
timeouts. Every arm used an independent pinned worktree, arm order alternated,
and the workload fixed `RUST_TEST_THREADS=1`.

The behavior invariant did not pass. Control made 20 provider requests while
Treatment made 23. Treatment recorded 342,947 total provider input tokens
(100,003 uncached) versus Control's 408,100 (142,884 uncached), but this is
not attributable to a reducer because output forwarding was passthrough. It is
therefore invalid to characterize these deltas as savings.

Follow-up source-byte telemetry found that a shared `CARGO_TARGET_DIR` could
survive worktree isolation. Results 003 therefore remain rejected and are not
evidence of a wrapper-induced trajectory difference. The runner now creates a
target directory inside every ephemeral arm state; it still persists no
commands, paths, stdout, stderr, prompts, or provider bodies.
