# Tracepress Baseline serial_n3

Metadata-only report. No request/response payloads, raw fingerprints, headers, or prices are included; the ledger contains opaque storage identities for accounting.

## Manifest

- `tracepress_commit`: 70957bba
- `measurement_instrument_version`: 1
- `codex_version`: 0.154.0
- `model`: gpt-5.6-luna
- `reasoning_effort`: xhigh
- `transport`: chatgpt_codex_subscription
- `analysis_version`: 1
- `provider_parser_version`: 1
- `usage_normalizer_version`: 1
- `fingerprint_version`: 1
- `detector_version`: 1
- `token_estimator_version`: 1

## Dataset

- Sessions: 3 total; 3 closed.
- Requests: 10.
- Request kinds: `{"turn": 10}`.

## Measurement quality

- Analysis: 100.00% (10/10).
- Request ledger: 10 eligible; 10 complete; 0 partial; 0 dropped.
- Measurement integrity: `failed`; conflicts: 1; unmatched drop events: 1.
- Auxiliary drops: 1 events / 3 work units.
- Correlation: 100.00% (10/10).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 64.35%.
- Semantic coverage mean: 95.02%.
- Provider observation partial: 7.

## Provider usage

- Input: 237,189; cached: 185,856; uncached: 51,333.
- Output: 4,559; reasoning: 2,892.
- Cache ratio: 78.36%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 10}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| plain_text | 106,862 | 40.74% | 47 |
| json | 94,758 | 36.13% | 14 |
| unknown | 59,836 | 22.81% | 145 |
| source_code | 830 | 0.32% | 10 |

## Repetition and exposure

- Exact repeated blocks: 138; semantic: 76.
- Exact repeated token share: 65.67%; semantic: 44.11%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 59,836 estimated tokens (22.81%).

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[84589.0, 96573.5, 132960.75]`.
- Context-size quartile boundaries: `[17390.0, 20800.0, 34034.25]`.
- Unavailable dimensions: `["workload", "concurrency_mode"]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.113159 | 40.74% | 6.6568 | 3.1333 |
| 2 | json | 0.014138 | 36.13% | unknown | 2 |
| 3 | unknown | 0.004759 | 22.81% | 6.7503 | 2.8429 |
| 4 | source_code | 0.000430 | 0.32% | 6.6667 | 3.3333 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Drop diagnostics

Metadata-only diagnostics; unavailable request/forward identities remain `unknown`.

| Event | Provider request | Forward | Session | Kind | Bytes | Reason | Classification | Work |
| ---: | --- | --- | --- | --- | ---: | --- | --- | ---: |
| 95 | unknown | unknown | 01a0913d-44a3-79c3-b1f3-11db07969cfa | unknown | unknown | correlation_degraded | auxiliary | 3 |

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 3 | 3 | 0 | 0 | 100.00% | 0.00% |
| q2 | 2 | 2 | 0 | 0 | 100.00% | 0.00% |
| q3 | 2 | 2 | 0 | 0 | 100.00% | 0.00% |
| q4 | 3 | 3 | 0 | 0 | 100.00% | 0.00% |
