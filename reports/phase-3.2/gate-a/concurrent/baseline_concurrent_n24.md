# Tracepress Baseline concurrent_n24

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

- Sessions: 24 total; 24 closed.
- Requests: 121.
- Request kinds: `{"turn": 121}`.

## Measurement quality

- Analysis: 95.87% (116/121).
- Request ledger: 121 eligible; 116 complete; 0 partial; 5 dropped.
- Measurement integrity: `failed`; conflicts: 15; unmatched drop events: 15.
- Auxiliary drops: 15 events / 62 work units.
- Correlation: 100.00% (116/116).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 65.20%.
- Semantic coverage mean: 95.45%.
- Provider observation partial: 96.

## Provider usage

- Input: 3,393,947; cached: 2,724,608; uncached: 669,339.
- Output: 39,052; reasoning: 24,549.
- Cache ratio: 80.28%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 116}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| json | 1,912,886 | 49.52% | 248 |
| plain_text | 1,241,683 | 32.14% | 527 |
| unknown | 695,326 | 18.00% | 1823 |
| source_code | 13,037 | 0.34% | 143 |

## Repetition and exposure

- Exact repeated blocks: 2,025; semantic: 1,010.
- Exact repeated token share: 71.52%; semantic: 39.42%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 695,326 estimated tokens (18.00%).

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[90452.0, 128778.0, 164342.0]`.
- Context-size quartile boundaries: `[17807.0, 30333.0, 46713.0]`.
- Unavailable dimensions: `["workload", "concurrency_mode"]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.083241 | 32.14% | 9.6637 | 4.6079 |
| 2 | json | 0.011407 | 49.52% | unknown | 2.6957 |
| 3 | unknown | 0.003349 | 18.00% | 9.7862 | 4.0048 |
| 4 | source_code | 0.000450 | 0.34% | 11.3780 | 4.3898 |

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
| 95 | unknown | unknown | 01a09120-2e52-78e1-9f05-a28066fd02e3 | unknown | unknown | correlation_degraded | auxiliary | 6 |
| 218 | unknown | unknown | 01a09123-7299-7872-baeb-51c08e56bc02 | unknown | unknown | correlation_degraded | auxiliary | 2 |
| 315 | unknown | unknown | 01a09124-729c-7230-af94-145ce855aa05 | unknown | unknown | correlation_degraded | auxiliary | 1 |
| 316 | unknown | unknown | 01a09124-72a7-75a1-9880-d51890f3f0f6 | unknown | unknown | correlation_degraded | auxiliary | 2 |
| 375 | unknown | unknown | 01a09127-71e2-7b41-a4ab-5f0a6c99986f | unknown | unknown | correlation_degraded | auxiliary | 5 |
| 492 | unknown | unknown | 01a0912a-4fe3-7332-b34b-9735084fc7bb | unknown | unknown | correlation_degraded | auxiliary | 2 |
| 609 | unknown | unknown | 01a0912b-9dcf-78a2-bd7e-e1710e42200a | unknown | unknown | correlation_degraded | auxiliary | 1 |
| 696 | unknown | unknown | 01a0912c-b8fd-7101-852d-996b0b142699 | unknown | unknown | correlation_degraded | auxiliary | 1 |
| 723 | unknown | unknown | 01a0912b-9dca-74c0-a304-6bd2dcec2b3d | unknown | unknown | correlation_degraded | auxiliary | 5 |
| 814 | unknown | unknown | 01a0912d-dbf4-7493-9c4e-3846e8c437d0 | unknown | unknown | correlation_degraded | auxiliary | 3 |
| 921 | unknown | unknown | 01a0912f-9610-7391-b552-e2f998be8ab1 | unknown | unknown | correlation_degraded | auxiliary | 3 |
| 1,018 | unknown | unknown | 01a09131-89a3-7d01-a167-dd14d9c23201 | unknown | unknown | correlation_degraded | auxiliary | 2 |
| 1,154 | unknown | unknown | 01a09133-1474-74c2-95c1-356f47471f69 | unknown | unknown | correlation_degraded | auxiliary | 7 |
| 1,155 | unknown | unknown | 01a09133-1470-7510-8a71-8cd5e11fca45 | unknown | unknown | observer_backpressure | auxiliary | 14 |
| 1,156 | unknown | unknown | 01a09133-1470-7510-8a71-8cd5e11fca45 | unknown | unknown | correlation_degraded | auxiliary | 8 |
| unknown | 01a09135-32ad-7883-b815-ce4f24b40249 | unknown | 01a09133-1470-7510-8a71-8cd5e11fca45 | turn | 93,096 | missing_durable_snapshot | request_outcome | 1 |
| unknown | 01a09135-57b6-74c1-8ff5-69aa7a06eb58 | unknown | 01a09133-1470-7510-8a71-8cd5e11fca45 | turn | 107,070 | missing_durable_snapshot | request_outcome | 1 |
| unknown | 01a09135-6d05-75d0-bbb1-866186f59afe | unknown | 01a09133-1470-7510-8a71-8cd5e11fca45 | turn | 149,332 | missing_durable_snapshot | request_outcome | 1 |
| unknown | 01a09135-b4e8-70d0-acb2-4ddd6a226fe0 | unknown | 01a09133-1470-7510-8a71-8cd5e11fca45 | turn | 176,323 | missing_durable_snapshot | request_outcome | 1 |
| unknown | 01a09135-e5ab-7611-b7c3-2264ff6fde0e | unknown | 01a09133-1470-7510-8a71-8cd5e11fca45 | turn | 186,826 | missing_durable_snapshot | request_outcome | 1 |

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 31 | 31 | 0 | 0 | 100.00% | 0.00% |
| q2 | 30 | 28 | 0 | 2 | 93.33% | 6.67% |
| q3 | 30 | 29 | 0 | 1 | 96.67% | 3.33% |
| q4 | 30 | 28 | 0 | 2 | 93.33% | 6.67% |
