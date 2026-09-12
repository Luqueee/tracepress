# Tracepress Baseline n10

Metadata-only report. No request/response payloads, raw fingerprints, headers, or prices are included; the ledger contains opaque storage identities for accounting.

## Manifest

- `tracepress_commit`: a15ac2dafa5074e447b8dd2811165f8f4be30c9f
- `runtime_commit`: a15ac2dafa5074e447b8dd2811165f8f4be30c9f
- `measurement_instrument_version`: 2
- `runtime_instrument_version`: 2
- `measurement_tooling_commit`: 348c5e5ea6950174f2de422a66a8f69df2383525
- `sidecar_schema_version`: 2
- `convergence_gate_version`: 2
- `workload_label_source`: harness_assigned
- `workload_taxonomy`: ['repo_exploration', 'large_search', 'bug_diagnosis', 'bug_fix', 'test_debugging', 'feature_implementation', 'refactor', 'code_review', 'dependency_investigation', 'long_running', 'compaction_investigation']
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
- Requests: 42.
- Request kinds: `{"turn": 42}`.

## Measurement quality

- Analysis: 100.00% (42/42).
- Request ledger: 42 eligible; 42 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (42/42).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 63.99%.
- Semantic coverage mean: 95.26%.
- Provider observation partial: 32.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | plain_text | 211/211 (100.00%) | 1,515,662/1,515,662 (100.00%) | 40.55% |
| detected_content_kind | json | 68/68 (100.00%) | 1,165,293/1,165,293 (100.00%) | 38.04% |
| detected_content_kind | unknown | 268/609 (44.01%) | 725,322/4,457,835 (16.27%) | 20.82% |
| detected_content_kind | source_code | 59/59 (100.00%) | 21,907/21,907 (100.00%) | 0.59% |
| context_block_kind | text | 470/470 (100.00%) | 2,212,374/2,212,374 (100.00%) | 60.87% |
| context_block_kind | tool_result | 68/68 (100.00%) | 1,165,293/1,165,293 (100.00%) | 38.04% |
| context_block_kind | tool_call | 68/68 (100.00%) | 50,517/50,517 (100.00%) | 1.09% |
| context_block_kind | message | 0/218 (0.00%) | 0/2,289,642 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/81 (0.00%) | 0/263,553 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/42 (0.00%) | 0/1,179,318 (0.00%) | 0.00% |
| context_origin | human_authored | 420/588 (71.43%) | 2,199,039/4,459,159 (49.32%) | 60.53% |
| context_origin | tool_generated | 68/68 (100.00%) | 1,165,293/1,165,293 (100.00%) | 38.04% |
| context_origin | agent_generated | 118/168 (70.24%) | 63,852/93,374 (68.38%) | 1.44% |
| context_origin | provider_managed | 0/81 (0.00%) | 0/263,553 (0.00%) | 0.00% |
| context_origin | unknown | 0/42 (0.00%) | 0/1,179,318 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 10.
- Admitted: 42; deferred: 0; processed: 42.
- High-water items P50/P90/P99: 1/1/1.
- High-water bytes P50/P90/P99: 55479.5000/63071/67355.9000.
- Analysis wait µs P50/P90/P99: 21.5000/29.2000/30.8200.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 42; ledger eligible: 42; delta: 0.

## Scheduler sidecar integrity

- Gate: `passed`; eligible requests: 42; complete snapshots: 42.
- Sidecar sessions: 10/10; missing: 0; capture issues: 0; counter mismatches: 0.
- Reported analysis seen: 42; unexplained counter delta: 0.
- Duplicate sidecar IDs: `[]`; unexpected IDs: `[]`; malformed records: 0.

| Session | Measurement run | PID | Sidecar file | Eligible | Complete snapshots | Admitted | Processed + drops | Status | Capture classification | Missing identity | Missing capture fields |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| 01a0964f-1016-7d42-9f92-5c2a934b44af | 3bc7b672-2a16-46a8-94e3-c3e1774c5b0e | 1,848,271 | 3bc7b672-2a16-46a8-94e3-c3e1774c5b0e.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a0964f-101b-7722-90f8-4ef13a3e5457 | de245a94-ed86-4f4f-b8a5-3c06c279f271 | 1,848,289 | de245a94-ed86-4f4f-b8a5-3c06c279f271.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09650-a12e-7673-af15-6aefc9675446 | 9b511826-84fc-49e3-97ae-e8a947994f48 | 1,855,056 | 9b511826-84fc-49e3-97ae-e8a947994f48.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09650-a13a-7571-985b-b255c67df625 | a169fad9-72af-4800-aa43-f8b68aad16b3 | 1,855,092 | a169fad9-72af-4800-aa43-f8b68aad16b3.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09652-421e-7812-93c0-9b1390dac848 | 315bd2c2-d313-4466-8944-30d32d0d8803 | 1,861,895 | 315bd2c2-d313-4466-8944-30d32d0d8803.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09652-4229-7b80-a0ca-7dc40ce4933d | fb743ce6-fa2a-4098-94c4-6f6c38b6d7e1 | 1,861,931 | fb743ce6-fa2a-4098-94c4-6f6c38b6d7e1.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a09653-8837-7693-93b7-5a15be0a90d0 | 92d8c0b8-cfb9-471e-a38c-07eac46058f9 | 1,869,507 | 92d8c0b8-cfb9-471e-a38c-07eac46058f9.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a09653-8844-7c90-8722-9c8b2d434d10 | 16efb104-d3f3-4db7-95db-cc66af628e0c | 1,869,550 | 16efb104-d3f3-4db7-95db-cc66af628e0c.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09655-77a4-7140-a148-86801bc2ce90 | e4162191-86e2-4d28-9655-6df85c9fccc2 | 1,880,534 | e4162191-86e2-4d28-9655-6df85c9fccc2.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09655-77b0-74d3-a008-5b7125dfd149 | fdfa05dd-6333-4044-8408-faa3c80ce559 | 1,880,572 | fdfa05dd-6333-4044-8408-faa3c80ce559.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |

## Provider usage

- Input: 1,096,110; cached: 817,664; uncached: 278,446.
- Output: 34,797; reasoning: 25,010.
- Cache ratio: 74.60%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 42}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| plain_text | 484,424 | 40.55% | 211 |
| json | 454,362 | 38.04% | 68 |
| unknown | 248,730 | 20.82% | 609 |
| source_code | 7,042 | 0.59% | 59 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| human_authored | text | plain_text | 479,476 | 40.14% | 1,497,975 |
| tool_generated | tool_result | json | 454,362 | 38.04% | 1,165,293 |
| human_authored | text | unknown | 240,072 | 20.10% | 692,580 |
| agent_generated | tool_call | unknown | 6,914 | 0.58% | 27,218 |
| agent_generated | tool_call | source_code | 3,556 | 0.30% | 13,423 |
| human_authored | text | source_code | 3,486 | 0.29% | 8,484 |
| agent_generated | tool_call | plain_text | 2,549 | 0.21% | 9,876 |
| agent_generated | text | plain_text | 2,399 | 0.20% | 7,811 |
| agent_generated | text | unknown | 1,744 | 0.15% | 5,524 |
| agent_generated | message | unknown | 0 | 0.00% | 29,522 |
| human_authored | message | unknown | 0 | 0.00% | 2,260,120 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 263,553 |
| unknown | unknown | unknown | 0 | 0.00% | 1,179,318 |

## Composition by workload

Workload labels are harness metadata; token shares use the estimable-token subset.

| Workload | Sessions | Session share | Estimated tokens | Workload token share | Bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| bug_diagnosis | 1 | 10.00% | 88,244 | 7.39% | 624,470 |
| code_review | 1 | 10.00% | 129,252 | 10.82% | 717,359 |
| compaction_investigation | 1 | 10.00% | 81,078 | 6.79% | 594,732 |
| dependency_investigation | 1 | 10.00% | 126,484 | 10.59% | 683,711 |
| feature_implementation | 1 | 10.00% | 93,649 | 7.84% | 626,947 |
| large_search | 2 | 20.00% | 241,427 | 20.21% | 1,511,195 |
| refactor | 1 | 10.00% | 161,490 | 13.52% | 806,241 |
| repo_exploration | 1 | 10.00% | 109,486 | 9.17% | 674,137 |
| test_debugging | 1 | 10.00% | 163,448 | 13.68% | 921,905 |

### Session-weighted detected content

Each session contributes one normalized composition before averaging.

| Detected kind | Mean session token share | Sessions with estimates |
| --- | ---: | ---: |
| json | 35.51% | 10 |
| plain_text | 42.27% | 10 |
| source_code | 0.65% | 10 |
| unknown | 21.56% | 10 |

### Workload x detected content

| Workload | Detected kind | Estimated tokens | Share within workload | Bytes |
| --- | --- | ---: | ---: | ---: |
| bug_diagnosis | plain_text | 46,514 | 52.71% | 145,680 |
| bug_diagnosis | unknown | 23,136 | 26.22% | 426,460 |
| bug_diagnosis | json | 17,535 | 19.87% | 48,901 |
| bug_diagnosis | source_code | 1,059 | 1.20% | 3,429 |
| code_review | json | 58,909 | 45.58% | 154,022 |
| code_review | plain_text | 45,839 | 35.46% | 143,385 |
| code_review | unknown | 23,383 | 18.09% | 416,474 |
| code_review | source_code | 1,121 | 0.87% | 3,478 |
| compaction_investigation | plain_text | 46,412 | 57.24% | 145,625 |
| compaction_investigation | unknown | 23,018 | 28.39% | 418,388 |
| compaction_investigation | json | 11,179 | 13.79% | 29,278 |
| compaction_investigation | source_code | 469 | 0.58% | 1,441 |
| dependency_investigation | json | 56,561 | 44.72% | 107,987 |
| dependency_investigation | plain_text | 45,945 | 36.32% | 143,547 |
| dependency_investigation | unknown | 23,646 | 18.69% | 431,369 |
| dependency_investigation | source_code | 332 | 0.26% | 808 |
| feature_implementation | plain_text | 45,887 | 49.00% | 143,435 |
| feature_implementation | json | 23,525 | 25.12% | 61,607 |
| feature_implementation | unknown | 23,040 | 24.60% | 417,214 |
| feature_implementation | source_code | 1,197 | 1.28% | 4,691 |
| large_search | plain_text | 103,220 | 42.75% | 322,636 |
| large_search | json | 83,285 | 34.50% | 222,224 |
| large_search | unknown | 53,398 | 22.12% | 961,826 |
| large_search | source_code | 1,524 | 0.63% | 4,509 |
| refactor | json | 89,645 | 55.51% | 227,579 |
| refactor | plain_text | 46,206 | 28.61% | 144,382 |
| refactor | unknown | 25,307 | 15.67% | 433,472 |
| refactor | source_code | 332 | 0.21% | 808 |
| repo_exploration | plain_text | 46,797 | 42.74% | 146,840 |
| repo_exploration | json | 38,986 | 35.61% | 107,904 |
| repo_exploration | unknown | 23,110 | 21.11% | 417,660 |
| repo_exploration | source_code | 593 | 0.54% | 1,733 |
| test_debugging | json | 74,737 | 45.73% | 205,791 |
| test_debugging | plain_text | 57,604 | 35.24% | 180,132 |
| test_debugging | unknown | 30,692 | 18.78% | 534,972 |
| test_debugging | source_code | 415 | 0.25% | 1,010 |

## Repetition and exposure

- Exact repeated blocks: 645; semantic: 347.
- Exact repeated token share: 66.55%; semantic: 46.29%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 248,730 estimated tokens (20.82%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| human_authored | text | plain_text | 40.14% | 365,316 | 365,316 | 4/5 |
| tool_generated | tool_result | json | 38.04% | 235,282 | 0 | 2/3 |
| human_authored | text | unknown | 20.10% | 182,912 | 182,912 | 4/5 |
| agent_generated | tool_call | unknown | 0.58% | 3,636 | 0 | 2/3.4000 |
| agent_generated | tool_call | source_code | 0.30% | 1,696 | 0 | 2/3 |
| human_authored | text | source_code | 0.29% | 2,656 | 2,656 | 4/5 |
| agent_generated | tool_call | plain_text | 0.21% | 1,304 | 0 | 2.5000/3 |
| agent_generated | text | plain_text | 0.20% | 1,239 | 1,239 | 2/3 |
| agent_generated | text | unknown | 0.15% | 889 | 889 | 2/3.3000 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 2/3 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 4/5 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 2/3 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 4/5 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[89413.25, 117502.5, 137584.5]`.
- Context-size quartile boundaries: `[17811.0, 26631.5, 34445.0]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "compaction_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 9, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 9, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 5, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 5, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 42, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 42, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.080019 | 40.55% | 8.3793 | 3.5877 |
| 2 | json | 0.007795 | 38.04% | unknown | 2.1250 |
| 3 | unknown | 0.003166 | 20.82% | 8.4555 | 3.3560 |
| 4 | source_code | 0.000785 | 0.59% | 12.6843 | 3.4828 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 11 | 11 | 0 | 0 | 100.00% | 0.00% |
| q2 | 10 | 10 | 0 | 0 | 100.00% | 0.00% |
| q3 | 10 | 10 | 0 | 0 | 100.00% | 0.00% |
| q4 | 11 | 11 | 0 | 0 | 100.00% | 0.00% |
