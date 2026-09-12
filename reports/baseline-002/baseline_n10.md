# Tracepress Baseline n10

Metadata-only report. No request/response payloads, raw fingerprints, headers, or prices are included; the ledger contains opaque storage identities for accounting.

## Manifest

- `tracepress_commit`: a15ac2dafa5074e447b8dd2811165f8f4be30c9f
- `measurement_instrument_version`: 2
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

- Sessions: 10 total; 10 closed.
- Requests: 159.
- Request kinds: `{"compaction_v2": 1, "turn": 158}`.

## Measurement quality

- Analysis: 100.00% (159/159).
- Request ledger: 159 eligible; 159 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (159/159).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 63.71%.
- Semantic coverage mean: 97.31%.
- Provider observation partial: 131.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 10.
- Admitted: 159; deferred: 0; processed: 159.
- High-water items P50/P90/P99: 2/2/2.
- High-water bytes P50/P90/P99: 194955.5000/445748.4000/513378.5400.
- Analysis wait µs P50/P90/P99: 793.5000/1375.8000/1641.4800.

## Provider usage

- Input: 12,746,362; cached: 11,875,584; uncached: 870,778.
- Output: 58,573; reasoning: 28,500.
- Cache ratio: 93.17%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 159}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| json | 16,497,949 | 84.21% | 1808 |
| plain_text | 1,823,178 | 9.31% | 723 |
| unknown | 1,194,237 | 6.10% | 5692 |
| source_code | 76,285 | 0.39% | 400 |

## Repetition and exposure

- Exact repeated blocks: 7,961; semantic: 1,754.
- Exact repeated token share: 93.09%; semantic: 13.24%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 1,194,237 estimated tokens (6.10%).

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[191927.0, 337896.0, 509763.5]`.
- Context-size quartile boundaries: `[57593.0, 112666.0, 163429.5]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 36, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 36, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 17, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 17, "name": "bug_fix", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 2, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 2, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 33, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 33, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 24, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 24, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 24, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 24, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 23, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 23, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 159, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 159, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `mixed`; requests: 1; V2: 1; legacy: 0.
- Trigger seen: 1; output seen: 1.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.007805 | 9.31% | 31.7179 | 14.1765 |
| 2 | json | 0.002839 | 84.21% | unknown | 12.2993 |
| 3 | source_code | 0.000543 | 0.39% | 107.8096 | 15.1081 |
| 4 | unknown | 0.000453 | 6.10% | 36.3875 | 13.4008 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 40 | 40 | 0 | 0 | 100.00% | 0.00% |
| q2 | 40 | 40 | 0 | 0 | 100.00% | 0.00% |
| q3 | 39 | 39 | 0 | 0 | 100.00% | 0.00% |
| q4 | 40 | 40 | 0 | 0 | 100.00% | 0.00% |
