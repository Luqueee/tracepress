# Tracepress Baseline-003 N40 Collection Report

This is a collection and integrity report for the tenth additional batch. It is not the canonical cumulative `baseline_n40` analysis because the SQLite database that produced the published N30 report is no longer available in the isolated workspace.

## Frozen instrument

- Runtime commit: `a15ac2dafa5074e447b8dd2811165f8f4be30c9f`
- Measurement tooling commit: `348c5e5ea6950174f2de422a66a8f69df2383525`
- Sidecar schema: `2`
- Codex: `0.154.0`
- Model/reasoning: `gpt-5.6-luna` / `xhigh`
- Transport: `chatgpt_codex_subscription`

## New batch

- Valid sessions: `10`
- Valid provider requests: `63`
- Complete analyses: `63/63`
- Correlated requests: `63/63`
- Partial analyses: `0`
- True analysis drops: `0`
- Backlog-capacity drops: `0`
- Forwarding errors: `0`
- Malformed contexts: `0`
- Sidecar captures: `10/10`
- Sidecar counter mismatches: `0`
- Excluded harness attempts: `1` (`duplicate_context_counters`)

The excluded attempt remains outside the cohort and is not used in the report.

## Scheduler

- Admitted: `63`
- Processed: `63`
- Deferred: `0`
- Final queue: `0 items / 0 bytes`
- Maximum high-water observed in the batch: `1 item / 92,996 bytes`
- Maximum analysis wait observed: `51 us`

The merged N30+N40 sidecar artifact contains `40` identity-bound complete session rows and `205` admitted/processed requests with no counter mismatch.

## New-batch measurements

Composition is weighted over locally estimable tokens:

| Detected content | Estimated-token share |
| --- | ---: |
| JSON | `56.20%` |
| Plain text | `28.36%` |
| Unknown | `14.98%` |
| Source code | `0.46%` |

- Estimation coverage by blocks: `64.61%`
- Provider input: `2,090,076`
- Cached input: `1,633,536`
- Uncached input: `456,540`
- Provider cache ratio: `78.16%`
- Provider output: `52,377`
- Reasoning output: `38,632`
- Exact repeated estimated-token share: `73.91%`
- Semantic repeated estimated-token share: `35.90%`
- Token reconciliation: unavailable (`missing_local_estimate`)
- Compaction requests: `0`

## Cumulative accounting visible from published artifacts

Combining the published N30 report with this independently analyzed N40 batch gives:

- Sessions: `40`
- Provider requests: `205`
- Complete analyses: `205/205`
- Correlated requests: `205/205`
- True drops: `0`
- Outcome conflicts: `0`
- Unmatched analysis events: `0`
- Forwarding errors: `0`

The additive context-mass view is descriptive only; it is not used as a convergence input:

- JSON: `45.48%` of the combined estimable-token mass
- Plain text: `35.52%`
- Unknown: `18.46%`
- Source code: `0.54%`
- Provider cache ratio: `77.04%`
- Exact repetition lower bound: `70.50%`
- Semantic repetition lower bound: `42.88%`

The repetition values are lower bounds because cross-boundary fingerprint matches between the historical N30 and new batch cannot be reconstructed without the original N30 SQLite. Quantiles and exact cumulative cross-request persistence likewise cannot be recomputed from the published aggregate report alone.

## Convergence status

`NOT_EVALUATED` for the cumulative N40 cohort.

The N40 batch passed its isolated integrity gate, but a canonical cumulative N40 report requires the original N30 SQLite (or a separately archived request/block-level dataset). Running the convergence script against the ten-session batch would be methodologically invalid because it would label an incremental batch as cumulative N40.

## Required follow-up

Archive the N30 SQLite/request-level dataset before the next adaptive batch, or start a new fully self-contained cohort. Do not claim `CONVERGED` from this collection artifact and do not select Phase 4 compressors from the additive summary alone.
