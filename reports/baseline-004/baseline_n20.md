# Tracepress Baseline n20

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

- Sessions: 20 total; 20 closed.
- Requests: 126.
- Request kinds: `{"turn": 126}`.

## Measurement quality

- Analysis: 100.00% (126/126).
- Request ledger: 126 eligible; 126 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (126/126).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 64.54%.
- Semantic coverage mean: 95.79%.
- Provider observation partial: 106.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | json | 338/338 (100.00%) | 6,263,289/6,263,289 (100.00%) | 52.23% |
| detected_content_kind | plain_text | 664/664 (100.00%) | 4,556,860/4,556,860 (100.00%) | 31.03% |
| detected_content_kind | unknown | 931/2092 (44.50%) | 2,258,201/13,686,033 (16.50%) | 16.26% |
| detected_content_kind | source_code | 180/180 (100.00%) | 72,381/72,381 (100.00%) | 0.48% |
| context_block_kind | tool_result | 338/338 (100.00%) | 6,263,289/6,263,289 (100.00%) | 52.23% |
| context_block_kind | text | 1437/1437 (100.00%) | 6,647,466/6,647,466 (100.00%) | 46.56% |
| context_block_kind | tool_call | 338/338 (100.00%) | 239,976/239,976 (100.00%) | 1.22% |
| context_block_kind | message | 0/681 (0.00%) | 0/6,887,972 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/354 (0.00%) | 0/1,001,906 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/126 (0.00%) | 0/3,537,954 (0.00%) | 0.00% |
| context_origin | tool_generated | 338/338 (100.00%) | 6,263,289/6,263,289 (100.00%) | 52.23% |
| context_origin | human_authored | 1260/1764 (71.43%) | 6,597,676/13,378,536 (49.32%) | 46.23% |
| context_origin | agent_generated | 515/692 (74.42%) | 289,766/396,878 (73.01%) | 1.55% |
| context_origin | provider_managed | 0/354 (0.00%) | 0/1,001,906 (0.00%) | 0.00% |
| context_origin | unknown | 0/126 (0.00%) | 0/3,537,954 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 20.
- Admitted: 126; deferred: 0; processed: 126.
- High-water items P50/P90/P99: 1/1/1.
- High-water bytes P50/P90/P99: 67263/82263.6000/86353.7400.
- Analysis wait µs P50/P90/P99: 36/47.3000/50.8100.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 126; ledger eligible: 126; delta: 0.

## Scheduler sidecar integrity

- Gate: `passed`; eligible requests: 126; complete snapshots: 126.
- Sidecar sessions: 20/20; missing: 0; capture issues: 0; counter mismatches: 0.
- Reported analysis seen: 126; unexplained counter delta: 0.
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
| 01a09aad-5ad7-7862-997f-430a78025615 | 2a4d99da-12e3-42c7-9d60-b719e84dc5e6 | 159,717 | 2a4d99da-12e3-42c7-9d60-b719e84dc5e6.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09aad-5ad7-7862-997f-43117f3b617a | 8984ab01-910f-4cb2-a6f1-2e59750a542b | 159,700 | 8984ab01-910f-4cb2-a6f1-2e59750a542b.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09aaf-7c4e-72f0-9fba-46898b5bee81 | 0863aedb-b2c0-4098-bf2e-c1809709cb53 | 171,151 | 0863aedb-b2c0-4098-bf2e-c1809709cb53.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09aaf-7c4e-72f0-9fba-469de43d850f | 055d07f7-b5db-4114-9530-35fbe3aa5532 | 171,152 | 055d07f7-b5db-4114-9530-35fbe3aa5532.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09ab1-2e09-7c60-bab5-48f37199affb | b205659c-7c3f-46db-a0db-19033b887f76 | 185,838 | b205659c-7c3f-46db-a0db-19033b887f76.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ab1-2e0e-76e3-891b-b4e4db6ad620 | a997351d-d2cf-48b6-9784-eb6bea926a5b | 185,856 | a997351d-d2cf-48b6-9784-eb6bea926a5b.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ab3-83f4-7313-bf2a-dec6824f0359 | a669c41d-0009-4880-8d1d-c004fea6fbb8 | 195,024 | a669c41d-0009-4880-8d1d-c004fea6fbb8.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ab3-83fb-77e2-add8-51902a2637b3 | 4e9e2318-a8d7-4e11-bbf7-2ea9cc626fdf | 195,045 | 4e9e2318-a8d7-4e11-bbf7-2ea9cc626fdf.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09ab6-8a5f-74a1-b849-46c63d09877c | c291b16b-ab38-432f-bd28-0b9884a08e26 | 206,365 | c291b16b-ab38-432f-bd28-0b9884a08e26.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ab6-8a60-70e3-a06a-00ddaa94c7ca | ab3920bd-897d-48ad-99e8-ff6a9b35942a | 206,374 | ab3920bd-897d-48ad-99e8-ff6a9b35942a.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |

## Provider usage

- Input: 3,979,224; cached: 3,089,920; uncached: 889,304.
- Output: 112,834; reasoning: 83,212.
- Cache ratio: 77.65%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 126}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| json | 2,450,746 | 52.23% | 338 |
| plain_text | 1,456,201 | 31.03% | 664 |
| unknown | 762,987 | 16.26% | 2092 |
| source_code | 22,684 | 0.48% | 180 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| tool_generated | tool_result | json | 2,450,746 | 52.23% | 6,263,289 |
| human_authored | text | plain_text | 1,438,649 | 30.66% | 4,494,484 |
| human_authored | text | unknown | 720,216 | 15.35% | 2,077,740 |
| agent_generated | tool_call | unknown | 37,405 | 0.80% | 163,208 |
| agent_generated | tool_call | source_code | 12,226 | 0.26% | 46,929 |
| human_authored | text | source_code | 10,458 | 0.22% | 25,452 |
| agent_generated | text | plain_text | 10,049 | 0.21% | 32,537 |
| agent_generated | tool_call | plain_text | 7,503 | 0.16% | 29,839 |
| agent_generated | text | unknown | 5,366 | 0.11% | 17,253 |
| agent_generated | message | unknown | 0 | 0.00% | 107,112 |
| human_authored | message | unknown | 0 | 0.00% | 6,780,860 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 1,001,906 |
| unknown | unknown | unknown | 0 | 0.00% | 3,537,954 |

## Composition by workload

Workload labels are harness metadata; token shares use the estimable-token subset.

| Workload | Sessions | Session share | Estimated tokens | Workload token share | Bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| bug_diagnosis | 1 | 5.00% | 164,699 | 3.51% | 1,016,210 |
| bug_fix | 2 | 10.00% | 594,937 | 12.68% | 2,808,459 |
| code_review | 2 | 10.00% | 373,466 | 7.96% | 2,163,972 |
| compaction_investigation | 1 | 5.00% | 300,640 | 6.41% | 1,475,848 |
| dependency_investigation | 2 | 10.00% | 282,068 | 6.01% | 1,936,319 |
| feature_implementation | 2 | 10.00% | 509,380 | 10.85% | 2,578,574 |
| large_search | 2 | 10.00% | 385,047 | 8.21% | 2,154,557 |
| long_running | 2 | 10.00% | 759,993 | 16.20% | 3,461,864 |
| refactor | 2 | 10.00% | 500,121 | 10.66% | 2,478,283 |
| repo_exploration | 2 | 10.00% | 354,545 | 7.56% | 2,027,631 |
| test_debugging | 2 | 10.00% | 467,722 | 9.97% | 2,476,846 |

### Session-weighted detected content

Each session contributes one normalized composition before averaging.

| Detected kind | Mean session token share | Sessions with estimates |
| --- | ---: | ---: |
| json | 48.25% | 20 |
| plain_text | 33.64% | 20 |
| source_code | 0.50% | 20 |
| unknown | 17.61% | 20 |

### Workload x detected content

| Workload | Detected kind | Estimated tokens | Share within workload | Bytes |
| --- | --- | ---: | ---: | ---: |
| bug_diagnosis | plain_text | 68,861 | 41.81% | 215,185 |
| bug_diagnosis | json | 58,453 | 35.49% | 155,777 |
| bug_diagnosis | unknown | 36,887 | 22.40% | 644,036 |
| bug_diagnosis | source_code | 498 | 0.30% | 1,212 |
| bug_fix | json | 362,771 | 60.98% | 923,722 |
| bug_fix | plain_text | 151,833 | 25.52% | 475,693 |
| bug_fix | unknown | 76,998 | 12.94% | 1,397,512 |
| bug_fix | source_code | 3,335 | 0.56% | 11,532 |
| code_review | json | 160,609 | 43.00% | 417,308 |
| code_review | plain_text | 137,872 | 36.92% | 430,687 |
| code_review | unknown | 73,613 | 19.71% | 1,311,899 |
| code_review | source_code | 1,372 | 0.37% | 4,078 |
| compaction_investigation | json | 176,639 | 58.75% | 455,729 |
| compaction_investigation | plain_text | 80,836 | 26.89% | 253,573 |
| compaction_investigation | unknown | 41,982 | 13.96% | 762,205 |
| compaction_investigation | source_code | 1,183 | 0.39% | 4,341 |
| dependency_investigation | plain_text | 138,712 | 49.18% | 434,825 |
| dependency_investigation | unknown | 71,691 | 25.42% | 1,315,894 |
| dependency_investigation | json | 70,467 | 24.98% | 182,383 |
| dependency_investigation | source_code | 1,198 | 0.42% | 3,217 |
| feature_implementation | json | 277,244 | 54.43% | 697,836 |
| feature_implementation | plain_text | 149,197 | 29.29% | 466,254 |
| feature_implementation | unknown | 81,293 | 15.96% | 1,409,419 |
| feature_implementation | source_code | 1,646 | 0.32% | 5,065 |
| large_search | json | 171,061 | 44.43% | 421,421 |
| large_search | plain_text | 138,008 | 35.84% | 431,139 |
| large_search | unknown | 73,509 | 19.09% | 1,294,956 |
| large_search | source_code | 2,469 | 0.64% | 7,041 |
| long_running | json | 489,413 | 64.40% | 1,228,844 |
| long_running | plain_text | 175,722 | 23.12% | 550,287 |
| long_running | unknown | 91,350 | 12.02% | 1,670,857 |
| long_running | source_code | 3,508 | 0.46% | 11,876 |
| refactor | json | 287,447 | 57.48% | 751,957 |
| refactor | plain_text | 138,283 | 27.65% | 432,957 |
| refactor | unknown | 71,762 | 14.35% | 1,284,470 |
| refactor | source_code | 2,629 | 0.53% | 8,899 |
| repo_exploration | json | 159,228 | 44.91% | 433,514 |
| repo_exploration | plain_text | 126,602 | 35.71% | 395,696 |
| repo_exploration | unknown | 66,939 | 18.88% | 1,193,798 |
| repo_exploration | source_code | 1,776 | 0.50% | 4,623 |
| test_debugging | json | 237,414 | 50.76% | 594,798 |
| test_debugging | plain_text | 150,275 | 32.13% | 470,564 |
| test_debugging | unknown | 76,963 | 16.45% | 1,400,987 |
| test_debugging | source_code | 3,070 | 0.66% | 10,497 |

## Repetition and exposure

- Exact repeated blocks: 2,544; semantic: 1,187.
- Exact repeated token share: 73.10%; semantic: 39.12%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 762,987 estimated tokens (16.26%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| tool_generated | tool_result | json | 52.23% | 1,557,875 | 0 | 3/5 |
| human_authored | text | plain_text | 30.66% | 1,210,295 | 1,210,295 | 6/7 |
| human_authored | text | unknown | 15.35% | 605,896 | 605,896 | 6/7 |
| agent_generated | tool_call | unknown | 0.80% | 24,624 | 0 | 3/5 |
| agent_generated | tool_call | source_code | 0.26% | 7,325 | 0 | 3/5 |
| human_authored | text | source_code | 0.22% | 8,798 | 8,798 | 6/7 |
| agent_generated | text | plain_text | 0.21% | 7,475 | 7,475 | 5/6 |
| agent_generated | tool_call | plain_text | 0.16% | 4,669 | 0 | 3/5.6000 |
| agent_generated | text | unknown | 0.11% | 3,285 | 3,285 | 2/5 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 4/6 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 6/7 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 3/5 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 6/7 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[93250.0, 131482.5, 184863.75]`.
- Context-size quartile boundaries: `[18878.75, 31059.0, 52040.75]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "bug_fix", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 12, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 12, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 7, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 7, "name": "compaction_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 12, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 12, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 12, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 12, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 15, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 15, "name": "long_running", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 12, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 12, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 11, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 11, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 126, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 126, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.059231 | 31.03% | 12.5792 | 5.5365 |
| 2 | json | 0.006706 | 52.23% | unknown | 3.1887 |
| 3 | unknown | 0.002220 | 16.26% | 12.7883 | 4.7715 |
| 4 | source_code | 0.000630 | 0.48% | 19.9651 | 5.1864 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 32 | 32 | 0 | 0 | 100.00% | 0.00% |
| q2 | 31 | 31 | 0 | 0 | 100.00% | 0.00% |
| q3 | 31 | 31 | 0 | 0 | 100.00% | 0.00% |
| q4 | 32 | 32 | 0 | 0 | 100.00% | 0.00% |
