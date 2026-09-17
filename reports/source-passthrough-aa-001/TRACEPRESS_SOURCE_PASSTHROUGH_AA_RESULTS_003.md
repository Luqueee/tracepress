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

Phase 5.0 must diagnose this wrapper-induced trajectory difference before
Shadow Source Mode or `cargo_test_v1` can be enabled. The runner now carries
allowlisted emitted-byte totals forward for that diagnosis; it still persists
no commands, paths, stdout, stderr, prompts, or provider bodies.
