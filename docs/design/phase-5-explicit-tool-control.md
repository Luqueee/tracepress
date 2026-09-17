# Phase 5.0.2: Explicit Tool Control

## Decision

Codex hook rewriting and PATH interception are not valid causal measurement surfaces for the
current runtime. Phase 5.0 therefore uses an explicit Tracepress tool command in both arms:

```text
tracepress tool cargo test
```

The control runs the wrapper in byte-faithful passthrough mode. The Shadow arm runs the same
wrapper and evaluates `cargo_test_v1`, but still emits the original stdout and stderr. This keeps
tool selection and the agent-visible command constant while measuring the reducer candidate.

No active source reduction is enabled in Phase 5.0.

## Contracts

Only standalone, allowlisted `cargo test` commands are admitted. Pipelines, redirections,
substitutions, heredocs, subshells, command lists, quoting, globs, and unknown syntax fail open and
remain outside the proxy. The proxy preserves stdout, stderr, exit status, and Unix termination
signals. `OutputContract::AgentReadable` is explicit; structural reduction is never assumed safe
for a machine consumer.

Each source receipt contains an opaque `SourceExecutionId` and the Tracepress session identity.
The session identity is the causal join from source execution and reducer decision to provider
requests and provider usage. Receipts contain allowlisted metadata only; commands, arguments,
working directories, stdout, stderr, and provider bodies are not persisted.

## Shadow candidate

`cargo_test_v1` removes passing-test and Cargo progress lines from an in-memory candidate while
retaining failures, summaries, warnings, errors, and other output. The original bytes are always
returned to Codex. Candidate evaluation records raw and candidate byte/token estimates, omitted
line counts, reducer latency, and recovery-hint overhead.

Never-worse accepts a candidate only when it is applicable and both candidate bytes and estimated
candidate tokens, including the recovery hint, are lower than raw. Rejection keeps raw bytes.

## Recovery design

Shadow mode does not create a recovery store because no bytes are hidden from the agent. An active
policy, if separately approved after Phase 5.0, must use the following design:

- generate an opaque, random `RecoveryId`; never expose a content hash or filesystem path;
- scope every recovery record to the current Tracepress session and enforce an expiry plus a short
  grace period;
- retain byte-faithful raw stdout and stderr only when an accepted active reduction hides content;
- bind recovery to `SourceExecutionId`, reducer id/version, output contract, and expiry without
  persisting the raw command, arguments, working directory, or content in observability rows;
- expose recovery only through an exact id lookup and delete expired payloads;
- count the complete agent-visible recovery hint in never-worse before an active candidate can be
  emitted.

`recovery_available` remains false throughout Shadow. The measured hint is hypothetical overhead,
not a claim that recovery is currently available.

## Experimental gates

The explicit A/A compares passthrough against identical passthrough on the same pinned public
repository, isolated per-arm worktrees and Cargo target directories, alternating arm order. It must
show complete instrumentation, identical source bytes/exit behavior, no hook activity, and no
systematic provider or trajectory change beyond measured run-to-run noise.

The Shadow pilot then compares passthrough with `cargo_test_v1` evaluation under the same controls.
It must show material candidate reduction, zero forwarding changes, zero exit/semantic mismatches,
bounded reducer overhead, and provider behavior consistent with the A/A noise envelope. Provider
reduction is expected to be zero because the candidate is not forwarded.

Phase 5.1 is a separate active experiment. It cannot begin merely because source candidate bytes
are smaller.
