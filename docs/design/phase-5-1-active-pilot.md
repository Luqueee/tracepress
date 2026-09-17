# Phase 5.1: `cargo_test_v1` active pilot

## Scope

Phase 5.1 activates one reducer for one command family in a bounded experiment. Both arms explicitly
invoke the same Tracepress wrapper. Control is passthrough; Treatment sets
`TRACEPRESS_SOURCE_REDUCER=cargo_test_v1_active`. No hook, PATH shim, pipeline, redirection, shell
composition, or second reducer participates.

## Active decision

Treatment executes the real command and builds the candidate with the complete agent-visible
recovery hint. It forwards the candidate only when:

1. the command was admitted as standalone `cargo test`;
2. the candidate is applicable;
3. candidate bytes and estimated tokens, including the hint, are both lower than raw;
4. a session-scoped recovery payload was written successfully.

Any failure returns raw stdout and stderr and records a bounded fail-open class. Process exit code
and Unix signal behavior remain owned by the passthrough runner.

## Recovery contract

The visible recovery identifier is 128 random bits encoded as 32 lowercase hexadecimal characters.
It is not derived from content, command, path, or session data. Payloads live under the private
Tracepress root, partitioned by validated session id, with 0700 directories and 0600 files.

The store keeps byte-faithful stdout and stderr separately, is capped at 8 MiB per execution, and
expires after one hour. Recall validates the token, requires the same Tracepress session, enforces
expiry and byte bounds, verifies stored byte counts, and replays the two streams without persisting
their content in telemetry. Expired exact-token directories are deleted opportunistically.

Observability records only recovery count and recovered byte counts. It does not persist the token,
raw command, arguments, working directory, or output.

## Experiment

The workload remains the pinned public ripgrep commit used by Phase 5.0. Ten paired Control and
Treatment tasks run in alternating order, each in a clean worktree and isolated Cargo target
directory. The agent must run the wrapper first, may recall or repeat it only when output is
insufficient, and may not inspect or modify the repository.

Task success is objective even when the pinned suite exits nonzero. Codex runs in JSON-output mode;
Tracepress transiently reads only the final structured `agent_message`, requires the exact sentinel
`TESTS_PASSED` or `TESTS_FAILED`, and compares it with the source process exit class. Only the two
bounded outcome classes and their equality are persisted. The message text is never written to the
report.

The report separates:

- source raw, candidate, and emitted bytes;
- provider total, cached, uncached, output, and reasoning tokens;
- task success, provider requests, tool calls, command retries, recoveries, and duration.

## Positive gate

The active reducer passes only when the full cohort has:

- material agent-visible source reduction;
- lower aggregate and paired-median uncached provider input;
- no task-success or process-contract regression;
- no fail-open executions;
- provider-request count no more than one request above Control across the cohort;
- no material retry or tool-call increase;
- recovery requests at or below 10% of filtered outputs.

A source-only reduction is insufficient. Failure of the positive gate disables the active policy
and records a negative result without weakening never-worse or recovery requirements.
