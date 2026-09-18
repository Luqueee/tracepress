# Phase 5.5: `rg_v1` Shadow

## Scope

Phase 5.5 admits standalone `rg` commands and evaluates one lossless, agent-readable projection.
Pipelines, redirections, substitutions, compound shell syntax, quoting, globs, and unknown syntax
remain fail-open. `TRACEPRESS_SOURCE_REDUCER=rg_v1_shadow` is explicit and always returns raw bytes
to the agent.

## Candidate contract

V1 accepts UTF-8 stdout only when every line has one unambiguous `path:line:content` boundary. It
groups consecutive matches under a file header and retains every path, line number, content byte,
line ending, and stderr byte. It performs no capping, truncation, deduplication, or recovery-backed
omission.

If a content line itself contains another `:digits:` shape, parsing is ambiguous and the entire
candidate fails closed to raw. The same applies to output without line numbers, invalid UTF-8, or
any unrecognized line. Never-worse still requires both fewer bytes and fewer estimated tokens.

## Workload selection

The first public smoke used `rg -n fn crates`. Four of 2,981 lines contained ambiguous numeric-colon
content, so the reducer correctly returned a raw-equivalent candidate. A content-free shape
characterization then compared a bounded set of simple patterns on the same pinned ripgrep commit.

`rg -n struct crates` was selected for the measured cohort because all 415 lines had an unambiguous
shape and the offline candidate exceeded the 20% materiality gate. The rejected ambiguous smoke is
retained as safety evidence rather than discarded.

## Gate

Control and Shadow run the same wrapper command in alternating, isolated sessions. Passing requires
at least 20% candidate reduction, exact raw forwarding, one accepted evaluation per Treatment,
objective task success, no retry/tool-call regression, and reducer latency below 50 ms. Provider
deltas remain A/A observations because Shadow never emits the candidate.
