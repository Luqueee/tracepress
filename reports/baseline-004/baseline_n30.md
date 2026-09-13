# Tracepress Baseline n30

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

- Sessions: 30 total; 30 closed.
- Requests: 193.
- Request kinds: `{"turn": 193}`.

## Measurement quality

- Analysis: 100.00% (193/193).
- Request ledger: 193 eligible; 193 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (193/193).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 64.55%.
- Semantic coverage mean: 95.82%.
- Provider observation partial: 163.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | json | 531/531 (100.00%) | 9,477,403/9,477,403 (100.00%) | 51.59% |
| detected_content_kind | plain_text | 1059/1059 (100.00%) | 7,002,950/7,002,950 (100.00%) | 31.51% |
| detected_content_kind | unknown | 1404/3196 (43.93%) | 3,447,551/20,936,302 (16.47%) | 16.43% |
| detected_content_kind | source_code | 269/269 (100.00%) | 104,807/104,807 (100.00%) | 0.47% |
| context_block_kind | tool_result | 531/531 (100.00%) | 9,477,403/9,477,403 (100.00%) | 51.59% |
| context_block_kind | text | 2201/2201 (100.00%) | 10,180,831/10,180,831 (100.00%) | 47.16% |
| context_block_kind | tool_call | 531/531 (100.00%) | 374,477/374,477 (100.00%) | 1.26% |
| context_block_kind | message | 0/1043 (0.00%) | 0/10,549,220 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/556 (0.00%) | 0/1,520,284 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/193 (0.00%) | 0/5,419,247 (0.00%) | 0.00% |
| context_origin | tool_generated | 531/531 (100.00%) | 9,477,403/9,477,403 (100.00%) | 51.59% |
| context_origin | human_authored | 1930/2702 (71.43%) | 10,104,391/20,489,399 (49.32%) | 46.82% |
| context_origin | agent_generated | 802/1073 (74.74%) | 450,917/615,129 (73.30%) | 1.59% |
| context_origin | provider_managed | 0/556 (0.00%) | 0/1,520,284 (0.00%) | 0.00% |
| context_origin | unknown | 0/193 (0.00%) | 0/5,419,247 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 30.
- Admitted: 193; deferred: 0; processed: 193.
- High-water items P50/P90/P99: 1/1/1.
- High-water bytes P50/P90/P99: 67622.5000/82263.6000/90039.4800.
- Analysis wait µs P50/P90/P99: 36/43.4000/50.7100.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 193; ledger eligible: 193; delta: 0.

## Scheduler sidecar integrity

- Gate: `passed`; eligible requests: 193; complete snapshots: 193.
- Sidecar sessions: 30/30; missing: 0; capture issues: 0; counter mismatches: 0.
- Reported analysis seen: 193; unexplained counter delta: 0.
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
| 01a09ab9-a523-7892-8fa5-02418f66db9f | b49461b8-0a75-4cc0-9746-8ab84b1d6620 | 219,843 | b49461b8-0a75-4cc0-9746-8ab84b1d6620.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ab9-a524-7413-b363-3c3554178fba | 61b1a0eb-a9b6-4d9c-ae28-1729881d406a | 219,845 | 61b1a0eb-a9b6-4d9c-ae28-1729881d406a.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09abb-5bfb-7823-9fa2-8be57e6e6b12 | 6ccfff4d-ef77-4d1e-8070-33db5d47c856 | 226,694 | 6ccfff4d-ef77-4d1e-8070-33db5d47c856.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09abb-5c01-7f93-adaa-dd2f406fca9d | a57d6780-2b4b-4ab7-a6e2-108c6c0d004b | 226,712 | a57d6780-2b4b-4ab7-a6e2-108c6c0d004b.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09abd-0f0d-7f51-9dd6-b48e2c3f494f | 214fddff-28ae-4484-bcf7-266ff4dc36fd | 233,644 | 214fddff-28ae-4484-bcf7-266ff4dc36fd.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09abd-0f11-7281-8d8d-0d838e9a6f36 | ea009a86-c450-4745-aa89-3bd143e5964d | 233,662 | ea009a86-c450-4745-aa89-3bd143e5964d.json | 8 | 8 | 8 | 8 | passed | complete | `[]` | `[]` |
| 01a09abf-9d37-7800-8a28-4931324dcaff | 19941431-1f90-4a68-86ca-c217d40af695 | 241,662 | 19941431-1f90-4a68-86ca-c217d40af695.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09abf-9d43-77c3-a1fc-ee71c6cc51b7 | 9e9cfa7c-d3cc-4590-b39b-50be39b2f2a2 | 241,698 | 9e9cfa7c-d3cc-4590-b39b-50be39b2f2a2.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09ac1-839b-7130-a378-2da689933510 | 9441e574-f46c-48b3-b2f4-08a3833ee768 | 247,953 | 9441e574-f46c-48b3-b2f4-08a3833ee768.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09ac1-83a3-74b2-ac5b-f55668b49da7 | 95be081d-af71-4aec-a620-d4ed86ab8017 | 247,978 | 95be081d-af71-4aec-a620-d4ed86ab8017.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |

## Provider usage

- Input: 6,075,946; cached: 4,753,152; uncached: 1,322,794.
- Output: 160,856; reasoning: 116,403.
- Cache ratio: 78.23%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 193}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| json | 3,660,539 | 51.59% | 531 |
| plain_text | 2,235,837 | 31.51% | 1059 |
| unknown | 1,166,048 | 16.43% | 3196 |
| source_code | 33,270 | 0.47% | 269 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| tool_generated | tool_result | json | 3,660,539 | 51.59% | 9,477,403 |
| human_authored | text | plain_text | 2,203,194 | 31.05% | 6,882,835 |
| human_authored | text | unknown | 1,103,188 | 15.55% | 3,182,570 |
| agent_generated | tool_call | unknown | 55,569 | 0.78% | 241,350 |
| agent_generated | tool_call | source_code | 17,251 | 0.24% | 65,821 |
| agent_generated | text | plain_text | 16,334 | 0.23% | 52,809 |
| agent_generated | tool_call | plain_text | 16,309 | 0.23% | 67,306 |
| human_authored | text | source_code | 16,019 | 0.23% | 38,986 |
| agent_generated | text | unknown | 7,291 | 0.10% | 23,631 |
| agent_generated | message | unknown | 0 | 0.00% | 164,212 |
| human_authored | message | unknown | 0 | 0.00% | 10,385,008 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 1,520,284 |
| unknown | unknown | unknown | 0 | 0.00% | 5,419,247 |

## Composition by workload

Workload labels are harness metadata; token shares use the estimable-token subset.

| Workload | Sessions | Session share | Estimated tokens | Workload token share | Bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| bug_diagnosis | 1 | 3.33% | 164,699 | 2.32% | 1,016,210 |
| bug_fix | 3 | 10.00% | 766,422 | 10.80% | 3,846,724 |
| code_review | 3 | 10.00% | 738,464 | 10.41% | 3,949,344 |
| compaction_investigation | 2 | 6.67% | 613,101 | 8.64% | 2,987,502 |
| dependency_investigation | 3 | 10.00% | 449,344 | 6.33% | 2,954,022 |
| feature_implementation | 3 | 10.00% | 774,593 | 10.92% | 3,973,970 |
| large_search | 3 | 10.00% | 664,678 | 9.37% | 3,580,976 |
| long_running | 3 | 10.00% | 947,127 | 13.35% | 4,637,657 |
| refactor | 3 | 10.00% | 699,049 | 9.85% | 3,602,133 |
| repo_exploration | 3 | 10.00% | 665,735 | 9.38% | 3,518,471 |
| test_debugging | 3 | 10.00% | 612,482 | 8.63% | 3,454,453 |

### Session-weighted detected content

Each session contributes one normalized composition before averaging.

| Detected kind | Mean session token share | Sessions with estimates |
| --- | ---: | ---: |
| json | 47.82% | 30 |
| plain_text | 33.98% | 30 |
| source_code | 0.49% | 30 |
| unknown | 17.71% | 30 |

### Workload x detected content

| Workload | Detected kind | Estimated tokens | Share within workload | Bytes |
| --- | --- | ---: | ---: | ---: |
| bug_diagnosis | plain_text | 68,861 | 41.81% | 215,185 |
| bug_diagnosis | json | 58,453 | 35.49% | 155,777 |
| bug_diagnosis | unknown | 36,887 | 22.40% | 644,036 |
| bug_diagnosis | source_code | 498 | 0.30% | 1,212 |
| bug_fix | json | 427,210 | 55.74% | 1,097,983 |
| bug_fix | plain_text | 221,117 | 28.85% | 692,241 |
| bug_fix | unknown | 114,262 | 14.91% | 2,043,756 |
| bug_fix | source_code | 3,833 | 0.50% | 12,744 |
| code_review | json | 383,884 | 51.98% | 1,012,772 |
| code_review | plain_text | 231,534 | 31.35% | 725,885 |
| code_review | unknown | 120,268 | 16.29% | 2,201,810 |
| code_review | source_code | 2,778 | 0.38% | 8,877 |
| compaction_investigation | json | 362,832 | 59.18% | 949,772 |
| compaction_investigation | plain_text | 161,217 | 26.30% | 504,760 |
| compaction_investigation | unknown | 87,288 | 14.24% | 1,527,215 |
| compaction_investigation | source_code | 1,764 | 0.29% | 5,755 |
| dependency_investigation | plain_text | 207,855 | 46.26% | 650,968 |
| dependency_investigation | json | 131,576 | 29.28% | 339,242 |
| dependency_investigation | unknown | 108,217 | 24.08% | 1,959,383 |
| dependency_investigation | source_code | 1,696 | 0.38% | 4,429 |
| feature_implementation | json | 417,102 | 53.85% | 1,081,661 |
| feature_implementation | plain_text | 229,505 | 29.63% | 717,187 |
| feature_implementation | unknown | 125,759 | 16.24% | 2,168,643 |
| feature_implementation | source_code | 2,227 | 0.29% | 6,479 |
| large_search | json | 324,921 | 48.88% | 829,226 |
| large_search | plain_text | 220,684 | 33.20% | 690,599 |
| large_search | unknown | 113,741 | 17.11% | 2,045,132 |
| large_search | source_code | 5,332 | 0.80% | 16,019 |
| long_running | json | 552,219 | 58.30% | 1,396,140 |
| long_running | plain_text | 257,903 | 27.23% | 809,160 |
| long_running | unknown | 131,650 | 13.90% | 2,414,157 |
| long_running | source_code | 5,355 | 0.57% | 18,200 |
| refactor | json | 379,733 | 54.32% | 1,000,444 |
| refactor | plain_text | 207,512 | 29.68% | 649,236 |
| refactor | unknown | 108,677 | 15.55% | 1,942,342 |
| refactor | source_code | 3,127 | 0.45% | 10,111 |
| repo_exploration | json | 346,326 | 52.02% | 912,156 |
| repo_exploration | plain_text | 209,296 | 31.44% | 657,067 |
| repo_exploration | unknown | 107,756 | 16.19% | 1,943,211 |
| repo_exploration | source_code | 2,357 | 0.35% | 6,037 |
| test_debugging | json | 276,283 | 45.11% | 702,230 |
| test_debugging | plain_text | 220,353 | 35.98% | 690,662 |
| test_debugging | unknown | 111,543 | 18.21% | 2,046,617 |
| test_debugging | source_code | 4,303 | 0.70% | 14,944 |

## Repetition and exposure

- Exact repeated blocks: 3,955; semantic: 1,828.
- Exact repeated token share: 73.27%; semantic: 39.78%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 1,166,048 estimated tokens (16.43%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| tool_generated | tool_result | json | 51.59% | 2,318,901 | 0 | 3/5 |
| human_authored | text | plain_text | 31.05% | 1,860,728 | 1,860,728 | 6/7 |
| human_authored | text | unknown | 15.55% | 931,708 | 931,708 | 6/7 |
| agent_generated | tool_call | unknown | 0.78% | 36,375 | 0 | 4/5 |
| agent_generated | tool_call | source_code | 0.24% | 10,169 | 0 | 3/5 |
| agent_generated | text | plain_text | 0.23% | 12,039 | 12,039 | 5/6 |
| agent_generated | tool_call | plain_text | 0.23% | 11,098 | 0 | 3/6 |
| human_authored | text | source_code | 0.23% | 13,529 | 13,529 | 6/7 |
| agent_generated | text | unknown | 0.10% | 4,769 | 4,769 | 3/6 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 4/6 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 6/7 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 3/5 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 6/7 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[96181.0, 129584.0, 183315.0]`.
- Context-size quartile boundaries: `[19109.0, 30858.0, 51011.0]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 6, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 6, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 19, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 19, "name": "bug_fix", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 20, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 20, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 14, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 14, "name": "compaction_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 18, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 18, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 20, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 20, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 19, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 19, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 22, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 22, "name": "long_running", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 18, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 18, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 18, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 18, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 19, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 19, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 193, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 193, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.061989 | 31.51% | 12.8485 | 5.5632 |
| 2 | json | 0.006797 | 51.59% | unknown | 3.2577 |
| 3 | unknown | 0.002386 | 16.43% | 13.0833 | 4.9394 |
| 4 | source_code | 0.000616 | 0.47% | 19.7948 | 5.2500 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 49 | 49 | 0 | 0 | 100.00% | 0.00% |
| q2 | 48 | 48 | 0 | 0 | 100.00% | 0.00% |
| q3 | 48 | 48 | 0 | 0 | 100.00% | 0.00% |
| q4 | 48 | 48 | 0 | 0 | 100.00% | 0.00% |
