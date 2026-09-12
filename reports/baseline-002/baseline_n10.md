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

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | json | 1808/1808 (100.00%) | 41,307,506/41,307,506 (100.00%) | 84.21% |
| detected_content_kind | plain_text | 723/723 (100.00%) | 5,698,997/5,698,997 (100.00%) | 9.31% |
| detected_content_kind | unknown | 2563/5692 (45.03%) | 3,811,773/22,115,636 (17.24%) | 6.10% |
| detected_content_kind | source_code | 400/400 (100.00%) | 255,950/255,950 (100.00%) | 0.39% |
| context_block_kind | tool_result | 1809/1809 (100.00%) | 41,307,833/41,307,833 (100.00%) | 84.21% |
| context_block_kind | text | 1876/1876 (100.00%) | 8,436,529/8,436,529 (100.00%) | 14.13% |
| context_block_kind | tool_call | 1809/1809 (100.00%) | 1,329,864/1,329,864 (100.00%) | 1.66% |
| context_block_kind | message | 0/922 (0.00%) | 0/8,760,300 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/2046 (0.00%) | 0/4,979,286 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/161 (0.00%) | 0/4,564,277 (0.00%) | 0.00% |
| context_origin | tool_generated | 1809/1809 (100.00%) | 41,307,833/41,307,833 (100.00%) | 84.21% |
| context_origin | human_authored | 1590/2226 (71.43%) | 8,335,381/16,901,883 (49.32%) | 13.98% |
| context_origin | agent_generated | 2095/2381 (87.99%) | 1,431,012/1,624,810 (88.07%) | 1.81% |
| context_origin | provider_managed | 0/2046 (0.00%) | 0/4,979,286 (0.00%) | 0.00% |
| context_origin | unknown | 0/161 (0.00%) | 0/4,564,277 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 10.
- Admitted: 159; deferred: 0; processed: 159.
- High-water items P50/P90/P99: 2/2/2.
- High-water bytes P50/P90/P99: 194955.5000/445748.4000/513378.5400.
- Analysis wait µs P50/P90/P99: 793.5000/1375.8000/1641.4800.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 159; ledger eligible: 159; delta: 0.

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

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| tool_generated | tool_result | json | 16,497,949 | 84.21% | 41,307,506 |
| human_authored | text | plain_text | 1,814,174 | 9.26% | 5,668,492 |
| human_authored | text | unknown | 910,728 | 4.65% | 2,634,771 |
| agent_generated | tool_call | unknown | 262,061 | 1.34% | 1,106,032 |
| agent_generated | tool_call | source_code | 63,088 | 0.32% | 223,832 |
| agent_generated | text | unknown | 21,448 | 0.11% | 70,970 |
| human_authored | text | source_code | 13,197 | 0.07% | 32,118 |
| agent_generated | text | plain_text | 8,987 | 0.05% | 30,178 |
| tool_generated | tool_result | plain_text | 17 | 0.00% | 327 |
| agent_generated | message | unknown | 0 | 0.00% | 193,798 |
| human_authored | message | unknown | 0 | 0.00% | 8,566,502 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 4,979,286 |
| unknown | unknown | unknown | 0 | 0.00% | 4,564,277 |

## Repetition and exposure

- Exact repeated blocks: 7,961; semantic: 1,754.
- Exact repeated token share: 93.09%; semantic: 13.24%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 1,194,237 estimated tokens (6.10%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| tool_generated | tool_result | json | 84.21% | 15,345,078 | 0 | 11/25.4000 |
| human_authored | text | plain_text | 9.26% | 1,700,060 | 1,700,060 | 16/34.5000 |
| human_authored | text | unknown | 4.65% | 853,602 | 853,602 | 17/33 |
| agent_generated | tool_call | unknown | 1.34% | 240,529 | 0 | 10/25 |
| agent_generated | tool_call | source_code | 0.32% | 57,381 | 0 | 13/29.6000 |
| agent_generated | text | unknown | 0.11% | 20,136 | 20,136 | 11/31.2000 |
| human_authored | text | source_code | 0.07% | 12,367 | 12,367 | 16.5000/33.3000 |
| agent_generated | text | plain_text | 0.05% | 8,156 | 8,156 | 6.5000/30 |
| tool_generated | tool_result | plain_text | 0.00% | 0 | 0 | 1/1 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 8.5000/30.9000 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 16/33 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 11/25.2000 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 16/33 |

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
