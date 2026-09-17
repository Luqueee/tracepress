# Phase 5.2: `cargo_check_v1` Shadow

## Scope

Phase 5.2 adds exactly one command family: standalone `cargo check`. Pipelines, redirections,
substitutions, compound commands, quoting, globs, and unknown shell syntax continue to fail open.
`cargo clippy`, `cargo build`, search, and Git commands remain unsupported.

The policy is opt-in through `TRACEPRESS_SOURCE_REDUCER=cargo_check_v1_shadow`. The wrapper executes
the real Cargo process once, captures its two byte streams, evaluates a candidate, records bounded
metadata, and returns the original stdout and stderr. Shadow cannot alter agent-visible bytes,
exit status, or signal behavior.

## Reducer contract

`cargo_check_v1` preserves stdout without interpretation. It parses stderr only when it is valid
UTF-8 and removes lines whose left-trimmed text starts with one of these Cargo progress prefixes:

- `Compiling `;
- `Checking `;
- `Downloading `;
- `Downloaded `;
- `Finished `.

Every other line is preserved, including warnings, errors, diagnostic locations, rendered source,
notes, help, build-script output, and final failure details. Invalid UTF-8 is non-applicable and
keeps the raw candidate.

The candidate includes the byte cost of a hypothetical recovery hint. Never-worse accepts it only
when both candidate bytes and the local token estimate are lower than raw. Phase 5.2 does not store
recovery payloads because raw remains visible.

## Experiment gate

The pinned public workload uses ten paired, alternating Control and Shadow sessions. Both arms run
the same absolute `tracepress tool cargo check` command in separate clean worktrees and separate
Cargo target directories. The objective evaluator compares the exact final `CHECK_PASSED` or
`CHECK_FAILED` sentinel with the source exit class without persisting agent text.

Passing requires material candidate reduction, one accepted evaluation per Treatment session,
byte-identical raw forwarding, objective task success, no retries or extra tool calls, and bounded
reducer latency. Provider deltas remain A/A noise because Shadow does not forward the candidate.
