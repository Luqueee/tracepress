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
- Requests: 63.
- Request kinds: `{"turn": 63}`.

## Measurement quality

- Analysis: 100.00% (63/63).
- Request ledger: 63 eligible; 63 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (63/63).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 64.61%.
- Semantic coverage mean: 95.77%.
- Provider observation partial: 53.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | json | 170/170 (100.00%) | 3,589,568/3,589,568 (100.00%) | 56.20% |
| detected_content_kind | plain_text | 306/306 (100.00%) | 2,267,768/2,267,768 (100.00%) | 28.36% |
| detected_content_kind | unknown | 482/1056 (45.64%) | 1,135,656/6,875,232 (16.52%) | 14.98% |
| detected_content_kind | source_code | 90/90 (100.00%) | 37,196/37,196 (100.00%) | 0.46% |
| context_block_kind | tool_result | 170/170 (100.00%) | 3,589,568/3,589,568 (100.00%) | 56.20% |
| context_block_kind | text | 708/708 (100.00%) | 3,323,151/3,323,151 (100.00%) | 42.72% |
| context_block_kind | tool_call | 170/170 (100.00%) | 117,469/117,469 (100.00%) | 1.08% |
| context_block_kind | message | 0/330 (0.00%) | 0/3,440,034 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/181 (0.00%) | 0/530,565 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/63 (0.00%) | 0/1,768,977 (0.00%) | 0.00% |
| context_origin | tool_generated | 170/170 (100.00%) | 3,589,568/3,589,568 (100.00%) | 56.20% |
| context_origin | human_authored | 630/882 (71.43%) | 3,300,089/6,691,791 (49.32%) | 42.44% |
| context_origin | agent_generated | 248/326 (76.07%) | 140,531/188,863 (74.41%) | 1.36% |
| context_origin | provider_managed | 0/181 (0.00%) | 0/530,565 (0.00%) | 0.00% |
| context_origin | unknown | 0/63 (0.00%) | 0/1,768,977 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 10.
- Admitted: 63; deferred: 0; processed: 63.
- High-water items P50/P90/P99: 1/1/1.
- High-water bytes P50/P90/P99: 67263/80621.1000/84137.3100.
- Analysis wait µs P50/P90/P99: 36/50.1000/50.9100.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 63; ledger eligible: 63; delta: 0.

## Scheduler sidecar integrity

- Gate: `passed`; eligible requests: 63; complete snapshots: 63.
- Sidecar sessions: 10/10; missing: 0; capture issues: 0; counter mismatches: 0.
- Reported analysis seen: 63; unexplained counter delta: 0.
- Duplicate sidecar IDs: `[]`; unexpected IDs: `[]`; malformed records: 0.

| Session | Measurement run | PID | Sidecar file | Eligible | Complete snapshots | Admitted | Processed + drops | Status | Capture classification | Missing identity | Missing capture fields |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| 01a09a7b-0742-7aa2-840d-13cae528d1ea | c613dfea-3810-47dd-b761-1bc06f86c753 | 29,337 | c613dfea-3810-47dd-b761-1bc06f86c753.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a09a7b-0742-7aa2-840d-13dff8fb28d8 | 14f2ae3d-9c9f-4bcd-9487-4690cc4a0dee | 29,336 | 14f2ae3d-9c9f-4bcd-9487-4690cc4a0dee.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09a7d-3341-7801-8cfb-e2b46ffa8800 | bb6165cc-9d46-4ff4-97dd-935762db9ecd | 35,800 | bb6165cc-9d46-4ff4-97dd-935762db9ecd.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09a7f-0bae-7791-9ad9-678460b81783 | 7e7f8593-c993-4c71-858c-a8ad786b9032 | 41,549 | 7e7f8593-c993-4c71-858c-a8ad786b9032.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09a7f-0bb3-7753-b409-d962192d8281 | 0c100fe8-261a-4b36-a4e3-e69cfaa0eb50 | 41,567 | 0c100fe8-261a-4b36-a4e3-e69cfaa0eb50.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09a80-fd5a-7f52-981e-ea2e3fbe5ed0 | 35d1893c-3935-4e41-8aff-21ed1e0b247a | 47,655 | 35d1893c-3935-4e41-8aff-21ed1e0b247a.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09a80-fd63-7251-b18f-df559a3dc8cc | 6b195e3b-dc7a-41d5-94c3-ba39cf6c6412 | 47,681 | 6b195e3b-dc7a-41d5-94c3-ba39cf6c6412.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09a82-f54f-78b2-8a36-c8b703f39667 | 2b7376e1-8fbf-4bf6-b3e8-f9ca080d0255 | 53,578 | 2b7376e1-8fbf-4bf6-b3e8-f9ca080d0255.json | 8 | 8 | 8 | 8 | passed | complete | `[]` | `[]` |
| 01a09a82-f555-77d0-a23e-feda8c3b2303 | 508bcee3-3d30-421e-ae83-7a1788a6b424 | 53,596 | 508bcee3-3d30-421e-ae83-7a1788a6b424.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09a85-703b-78d2-bf02-742af76becb0 | de5ea4dc-08cb-435c-a514-b4936c65290c | 60,863 | de5ea4dc-08cb-435c-a514-b4936c65290c.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |

## Provider usage

- Input: 2,090,076; cached: 1,633,536; uncached: 456,540.
- Output: 52,377; reasoning: 38,632.
- Cache ratio: 78.16%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 63}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| json | 1,436,903 | 56.20% | 170 |
| plain_text | 725,027 | 28.36% | 306 |
| unknown | 383,091 | 14.98% | 1056 |
| source_code | 11,652 | 0.46% | 90 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| tool_generated | tool_result | json | 1,436,903 | 56.20% | 3,589,568 |
| human_authored | text | plain_text | 719,689 | 28.15% | 2,248,493 |
| human_authored | text | unknown | 360,108 | 14.09% | 1,038,870 |
| agent_generated | tool_call | unknown | 19,567 | 0.77% | 85,684 |
| agent_generated | tool_call | source_code | 6,423 | 0.25% | 24,470 |
| human_authored | text | source_code | 5,229 | 0.20% | 12,726 |
| agent_generated | text | plain_text | 3,732 | 0.15% | 11,960 |
| agent_generated | text | unknown | 3,416 | 0.13% | 11,102 |
| agent_generated | tool_call | plain_text | 1,606 | 0.06% | 7,315 |
| agent_generated | message | unknown | 0 | 0.00% | 48,332 |
| human_authored | message | unknown | 0 | 0.00% | 3,391,702 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 530,565 |
| unknown | unknown | unknown | 0 | 0.00% | 1,768,977 |

## Composition by workload

Workload labels are harness metadata; token shares use the estimable-token subset.

| Workload | Sessions | Session share | Estimated tokens | Workload token share | Bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| bug_fix | 1 | 10.00% | 277,693 | 10.86% | 1,274,424 |
| code_review | 1 | 10.00% | 198,647 | 7.77% | 1,110,951 |
| compaction_investigation | 1 | 10.00% | 300,640 | 11.76% | 1,475,848 |
| dependency_investigation | 1 | 10.00% | 137,209 | 5.37% | 951,886 |
| feature_implementation | 1 | 10.00% | 255,220 | 9.98% | 1,232,081 |
| large_search | 1 | 10.00% | 233,344 | 9.13% | 1,172,493 |
| long_running | 1 | 10.00% | 417,856 | 16.34% | 1,868,833 |
| refactor | 1 | 10.00% | 297,779 | 11.65% | 1,366,952 |
| repo_exploration | 1 | 10.00% | 135,178 | 5.29% | 854,039 |
| test_debugging | 1 | 10.00% | 303,107 | 11.86% | 1,462,257 |

### Session-weighted detected content

Each session contributes one normalized composition before averaging.

| Detected kind | Mean session token share | Sessions with estimates |
| --- | ---: | ---: |
| json | 52.55% | 10 |
| plain_text | 30.79% | 10 |
| source_code | 0.49% | 10 |
| unknown | 16.16% | 10 |

### Workload x detected content

| Workload | Detected kind | Estimated tokens | Share within workload | Bytes |
| --- | --- | ---: | ---: | ---: |
| bug_fix | json | 171,222 | 61.66% | 412,630 |
| bug_fix | plain_text | 69,034 | 24.86% | 215,617 |
| bug_fix | unknown | 36,360 | 13.09% | 642,424 |
| bug_fix | source_code | 1,077 | 0.39% | 3,753 |
| code_review | json | 91,797 | 46.21% | 232,103 |
| code_review | plain_text | 69,053 | 34.76% | 215,759 |
| code_review | unknown | 36,923 | 18.59% | 660,223 |
| code_review | source_code | 874 | 0.44% | 2,866 |
| compaction_investigation | json | 176,639 | 58.75% | 455,729 |
| compaction_investigation | plain_text | 80,836 | 26.89% | 253,573 |
| compaction_investigation | unknown | 41,982 | 13.96% | 762,205 |
| compaction_investigation | source_code | 1,183 | 0.39% | 4,341 |
| dependency_investigation | plain_text | 69,634 | 50.75% | 218,931 |
| dependency_investigation | unknown | 35,232 | 25.68% | 649,322 |
| dependency_investigation | json | 31,643 | 23.06% | 81,628 |
| dependency_investigation | source_code | 700 | 0.51% | 2,005 |
| feature_implementation | json | 148,483 | 58.18% | 367,071 |
| feature_implementation | plain_text | 68,538 | 26.85% | 214,122 |
| feature_implementation | unknown | 37,134 | 14.55% | 647,237 |
| feature_implementation | source_code | 1,065 | 0.42% | 3,651 |
| large_search | json | 126,046 | 54.02% | 304,505 |
| large_search | plain_text | 69,024 | 29.58% | 215,643 |
| large_search | unknown | 36,303 | 15.56% | 646,516 |
| large_search | source_code | 1,971 | 0.84% | 5,829 |
| long_running | json | 273,437 | 65.44% | 662,573 |
| long_running | plain_text | 92,197 | 22.06% | 288,144 |
| long_running | unknown | 50,573 | 12.10% | 912,690 |
| long_running | source_code | 1,649 | 0.39% | 5,426 |
| refactor | json | 191,237 | 64.22% | 499,719 |
| refactor | plain_text | 68,906 | 23.14% | 215,358 |
| refactor | unknown | 37,138 | 12.47% | 650,663 |
| refactor | source_code | 498 | 0.17% | 1,212 |
| repo_exploration | plain_text | 57,474 | 42.52% | 179,665 |
| repo_exploration | json | 47,111 | 34.85% | 130,274 |
| repo_exploration | unknown | 29,315 | 21.69% | 540,689 |
| repo_exploration | source_code | 1,278 | 0.95% | 3,411 |
| test_debugging | json | 179,288 | 59.15% | 443,336 |
| test_debugging | plain_text | 80,331 | 26.50% | 250,956 |
| test_debugging | unknown | 42,131 | 13.90% | 763,263 |
| test_debugging | source_code | 1,357 | 0.45% | 4,702 |

## Repetition and exposure

- Exact repeated blocks: 1,263; semantic: 587.
- Exact repeated token share: 73.91%; semantic: 35.90%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 383,091 estimated tokens (14.98%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| tool_generated | tool_result | json | 56.20% | 954,046 | 0 | 3/5 |
| human_authored | text | plain_text | 28.15% | 605,455 | 605,455 | 6/7.1000 |
| human_authored | text | unknown | 14.09% | 302,948 | 302,948 | 6/7.1000 |
| agent_generated | tool_call | unknown | 0.77% | 12,966 | 0 | 3/5.2000 |
| agent_generated | tool_call | source_code | 0.25% | 3,583 | 0 | 3/4.1000 |
| human_authored | text | source_code | 0.20% | 4,399 | 4,399 | 6/7.1000 |
| agent_generated | text | plain_text | 0.15% | 2,967 | 2,967 | 5/6.3000 |
| agent_generated | text | unknown | 0.13% | 2,204 | 2,204 | 2.5000/5 |
| agent_generated | tool_call | plain_text | 0.06% | 989 | 0 | 2.5000/3.7000 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 4/6 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 6/7.1000 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 3/5 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 6/7.1000 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[92349.5, 142356.0, 199518.5]`.
- Context-size quartile boundaries: `[18696.5, 34297.0, 56582.0]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "bug_fix", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 7, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 7, "name": "compaction_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 8, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 8, "name": "long_running", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 5, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 5, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 7, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 7, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 63, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 63, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.053247 | 28.36% | 12.5953 | 5.8922 |
| 2 | json | 0.006702 | 56.20% | unknown | 3.2075 |
| 3 | unknown | 0.001887 | 14.98% | 12.7906 | 4.7661 |
| 4 | source_code | 0.000549 | 0.46% | 20.3386 | 5.1000 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 16 | 16 | 0 | 0 | 100.00% | 0.00% |
| q2 | 16 | 16 | 0 | 0 | 100.00% | 0.00% |
| q3 | 15 | 15 | 0 | 0 | 100.00% | 0.00% |
| q4 | 16 | 16 | 0 | 0 | 100.00% | 0.00% |
