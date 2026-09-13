# Tracepress Baseline n40

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

- Sessions: 40 total; 40 closed.
- Requests: 258.
- Request kinds: `{"turn": 258}`.

## Measurement quality

- Analysis: 100.00% (258/258).
- Request ledger: 258 eligible; 258 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (258/258).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 64.47%.
- Semantic coverage mean: 95.83%.
- Provider observation partial: 218.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | json | 712/712 (100.00%) | 11,886,323/11,886,323 (100.00%) | 49.83% |
| detected_content_kind | plain_text | 1423/1423 (100.00%) | 9,372,104/9,372,104 (100.00%) | 32.66% |
| detected_content_kind | unknown | 1873/4282 (43.74%) | 4,600,340/28,040,008 (16.41%) | 17.01% |
| detected_content_kind | source_code | 363/363 (100.00%) | 143,238/143,238 (100.00%) | 0.50% |
| context_block_kind | tool_result | 712/712 (100.00%) | 11,886,323/11,886,323 (100.00%) | 49.83% |
| context_block_kind | text | 2947/2947 (100.00%) | 13,609,774/13,609,774 (100.00%) | 48.84% |
| context_block_kind | tool_call | 712/712 (100.00%) | 505,908/505,908 (100.00%) | 1.32% |
| context_block_kind | message | 0/1399 (0.00%) | 0/14,103,770 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/752 (0.00%) | 0/2,091,516 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/258 (0.00%) | 0/7,244,382 (0.00%) | 0.00% |
| context_origin | tool_generated | 712/712 (100.00%) | 11,886,323/11,886,323 (100.00%) | 49.83% |
| context_origin | human_authored | 2580/3612 (71.43%) | 13,506,011/27,387,150 (49.32%) | 48.49% |
| context_origin | agent_generated | 1079/1446 (74.62%) | 609,671/832,302 (73.25%) | 1.67% |
| context_origin | provider_managed | 0/752 (0.00%) | 0/2,091,516 (0.00%) | 0.00% |
| context_origin | unknown | 0/258 (0.00%) | 0/7,244,382 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 40.
- Admitted: 258; deferred: 0; processed: 258.
- High-water items P50/P90/P99: 1/1/1.
- High-water bytes P50/P90/P99: 67622.5000/81620.5000/89580.6800.
- Analysis wait µs P50/P90/P99: 36/45.2000/50.6100.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 258; ledger eligible: 258; delta: 0.

## Scheduler sidecar integrity

- Gate: `passed`; eligible requests: 258; complete snapshots: 258.
- Sidecar sessions: 40/40; missing: 0; capture issues: 0; counter mismatches: 0.
- Reported analysis seen: 258; unexplained counter delta: 0.
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
| 01a09ac4-3253-7041-9baa-fc560783f02a | d71ac4fb-b73c-435f-9d61-f294a3d637e3 | 255,707 | d71ac4fb-b73c-435f-9d61-f294a3d637e3.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09ac4-3259-7492-976c-08d6da315ed4 | 3c1a491e-c9ad-45df-b596-d99c6b0995ea | 255,731 | 3c1a491e-c9ad-45df-b596-d99c6b0995ea.json | 8 | 8 | 8 | 8 | passed | complete | `[]` | `[]` |
| 01a09ac6-29e2-7633-9231-a8ab143c3266 | 73de2d8a-a26b-4b5d-b5c5-d4538774db5c | 262,334 | 73de2d8a-a26b-4b5d-b5c5-d4538774db5c.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ac6-29eb-7213-80d8-c988bb10c60f | 94d0dde8-9057-4932-918b-c912960c0095 | 262,362 | 94d0dde8-9057-4932-918b-c912960c0095.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ac8-8752-71e0-822a-d8e7ef84d6d1 | fc6a7e3f-ba0a-40cb-bc44-4a174ae11a62 | 269,579 | fc6a7e3f-ba0a-40cb-bc44-4a174ae11a62.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09ac8-8757-7851-87e6-3f642cef22c6 | c41053fa-19fb-459f-878d-2955a2c480cb | 269,597 | c41053fa-19fb-459f-878d-2955a2c480cb.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09aca-131e-7e93-8fe4-d5072529f562 | cc003905-0fb5-4200-beac-9ad2a2f5b144 | 275,196 | cc003905-0fb5-4200-beac-9ad2a2f5b144.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09aca-1325-7c83-a104-04409247124f | c52c4223-c590-450d-b589-8e7dca353886 | 275,219 | c52c4223-c590-450d-b589-8e7dca353886.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a09acb-cb21-7b32-992e-b8be709693d8 | 104e8b0d-4b14-4332-9fa9-f1553e49cf49 | 280,922 | 104e8b0d-4b14-4332-9fa9-f1553e49cf49.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a09acb-cb26-7f40-820b-976542e42def | 169d2862-3fd9-4c4f-ad1e-c1862ae078b4 | 280,941 | 169d2862-3fd9-4c4f-ad1e-c1862ae078b4.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |

## Provider usage

- Input: 7,911,876; cached: 6,267,392; uncached: 1,644,484.
- Output: 211,187; reasoning: 152,100.
- Cache ratio: 79.21%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 258}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| json | 4,563,897 | 49.83% | 712 |
| plain_text | 2,991,081 | 32.66% | 1423 |
| unknown | 1,557,400 | 17.01% | 4282 |
| source_code | 45,657 | 0.50% | 363 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| tool_generated | tool_result | json | 4,563,897 | 49.83% | 11,886,323 |
| human_authored | text | plain_text | 2,944,409 | 32.15% | 9,198,040 |
| human_authored | text | unknown | 1,475,141 | 16.11% | 4,255,855 |
| agent_generated | tool_call | unknown | 71,283 | 0.78% | 309,107 |
| agent_generated | tool_call | plain_text | 25,536 | 0.28% | 105,679 |
| agent_generated | tool_call | source_code | 24,243 | 0.26% | 91,122 |
| human_authored | text | source_code | 21,414 | 0.23% | 52,116 |
| agent_generated | text | plain_text | 21,136 | 0.23% | 68,385 |
| agent_generated | text | unknown | 10,976 | 0.12% | 35,378 |
| agent_generated | message | unknown | 0 | 0.00% | 222,631 |
| human_authored | message | unknown | 0 | 0.00% | 13,881,139 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 2,091,516 |
| unknown | unknown | unknown | 0 | 0.00% | 7,244,382 |

## Composition by workload

Workload labels are harness metadata; token shares use the estimable-token subset.

| Workload | Sessions | Session share | Estimated tokens | Workload token share | Bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| bug_diagnosis | 2 | 5.00% | 393,688 | 4.30% | 2,240,215 |
| bug_fix | 4 | 10.00% | 1,023,460 | 11.18% | 5,222,762 |
| code_review | 4 | 10.00% | 926,134 | 10.11% | 5,034,979 |
| compaction_investigation | 2 | 5.00% | 613,101 | 6.69% | 2,987,502 |
| dependency_investigation | 4 | 10.00% | 612,127 | 6.68% | 3,965,287 |
| feature_implementation | 4 | 10.00% | 956,705 | 10.45% | 5,043,693 |
| large_search | 4 | 10.00% | 909,735 | 9.93% | 5,012,019 |
| long_running | 4 | 10.00% | 1,162,376 | 12.69% | 5,935,797 |
| refactor | 4 | 10.00% | 874,526 | 9.55% | 4,656,107 |
| repo_exploration | 4 | 10.00% | 919,356 | 10.04% | 4,891,429 |
| test_debugging | 4 | 10.00% | 766,827 | 8.37% | 4,451,883 |

### Session-weighted detected content

Each session contributes one normalized composition before averaging.

| Detected kind | Mean session token share | Sessions with estimates |
| --- | ---: | ---: |
| json | 46.57% | 40 |
| plain_text | 34.80% | 40 |
| source_code | 0.52% | 40 |
| unknown | 18.11% | 40 |

### Workload x detected content

| Workload | Detected kind | Estimated tokens | Share within workload | Bytes |
| --- | --- | ---: | ---: | ---: |
| bug_diagnosis | json | 179,030 | 45.48% | 493,306 |
| bug_diagnosis | plain_text | 137,745 | 34.99% | 430,386 |
| bug_diagnosis | unknown | 75,085 | 19.07% | 1,311,031 |
| bug_diagnosis | source_code | 1,828 | 0.46% | 5,492 |
| bug_fix | json | 558,664 | 54.59% | 1,440,282 |
| bug_fix | plain_text | 303,483 | 29.65% | 951,768 |
| bug_fix | unknown | 155,781 | 15.22% | 2,812,762 |
| bug_fix | source_code | 5,532 | 0.54% | 17,950 |
| code_review | json | 466,116 | 50.33% | 1,233,534 |
| code_review | plain_text | 300,538 | 32.45% | 941,536 |
| code_review | unknown | 156,204 | 16.87% | 2,849,820 |
| code_review | source_code | 3,276 | 0.35% | 10,089 |
| compaction_investigation | json | 362,832 | 59.18% | 949,772 |
| compaction_investigation | plain_text | 161,217 | 26.30% | 504,760 |
| compaction_investigation | unknown | 87,288 | 14.24% | 1,527,215 |
| compaction_investigation | source_code | 1,764 | 0.29% | 5,755 |
| dependency_investigation | plain_text | 276,630 | 45.19% | 865,767 |
| dependency_investigation | json | 188,042 | 30.72% | 477,532 |
| dependency_investigation | unknown | 145,261 | 23.73% | 2,616,347 |
| dependency_investigation | source_code | 2,194 | 0.36% | 5,641 |
| feature_implementation | json | 492,342 | 51.46% | 1,286,166 |
| feature_implementation | plain_text | 298,981 | 31.25% | 934,996 |
| feature_implementation | unknown | 160,359 | 16.76% | 2,807,126 |
| feature_implementation | source_code | 5,023 | 0.53% | 15,405 |
| large_search | json | 426,633 | 46.90% | 1,086,513 |
| large_search | plain_text | 314,917 | 34.62% | 987,109 |
| large_search | unknown | 159,865 | 17.57% | 2,911,997 |
| large_search | source_code | 8,320 | 0.91% | 26,400 |
| long_running | json | 643,263 | 55.34% | 1,650,493 |
| long_running | plain_text | 341,164 | 29.35% | 1,072,874 |
| long_running | unknown | 172,013 | 14.80% | 3,192,816 |
| long_running | source_code | 5,936 | 0.51% | 19,614 |
| refactor | json | 448,701 | 51.31% | 1,179,866 |
| refactor | plain_text | 278,053 | 31.79% | 870,734 |
| refactor | unknown | 143,727 | 16.43% | 2,592,222 |
| refactor | source_code | 4,045 | 0.46% | 13,285 |
| repo_exploration | json | 473,848 | 51.54% | 1,258,206 |
| repo_exploration | plain_text | 289,166 | 31.45% | 906,540 |
| repo_exploration | unknown | 153,404 | 16.69% | 2,719,232 |
| repo_exploration | source_code | 2,938 | 0.32% | 7,451 |
| test_debugging | json | 324,426 | 42.31% | 830,653 |
| test_debugging | plain_text | 289,187 | 37.71% | 905,634 |
| test_debugging | unknown | 148,413 | 19.35% | 2,699,440 |
| test_debugging | source_code | 4,801 | 0.63% | 16,156 |

## Repetition and exposure

- Exact repeated blocks: 5,302; semantic: 2,448.
- Exact repeated token share: 73.42%; semantic: 41.22%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 1,557,400 estimated tokens (17.01%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| tool_generated | tool_result | json | 49.83% | 2,869,733 | 0 | 3/5 |
| human_authored | text | plain_text | 32.15% | 2,487,909 | 2,487,909 | 6/7 |
| human_authored | text | unknown | 16.11% | 1,246,442 | 1,246,442 | 6/7 |
| agent_generated | tool_call | unknown | 0.78% | 46,652 | 0 | 4/5 |
| agent_generated | tool_call | plain_text | 0.28% | 17,612 | 0 | 3/6 |
| agent_generated | tool_call | source_code | 0.26% | 14,275 | 0 | 3/5 |
| human_authored | text | source_code | 0.23% | 18,094 | 18,094 | 6/7 |
| agent_generated | text | plain_text | 0.23% | 15,677 | 15,677 | 5/6 |
| agent_generated | text | unknown | 0.12% | 7,209 | 7,209 | 3/6 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 4/6 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 6/7 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 3/5 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 6/7 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[96235.0, 128918.5, 176297.75]`.
- Context-size quartile boundaries: `[19460.25, 30220.0, 48720.0]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 12, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 12, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 26, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 26, "name": "bug_fix", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 26, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 26, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 14, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 14, "name": "compaction_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 24, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 24, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 26, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 26, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 27, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 27, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 29, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 29, "name": "long_running", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 24, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 24, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 25, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 25, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 25, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 25, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 258, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 258, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.063110 | 32.66% | 12.8943 | 5.5691 |
| 2 | json | 0.006436 | 49.83% | unknown | 3.2661 |
| 3 | unknown | 0.002409 | 17.01% | 13.0923 | 4.9247 |
| 4 | source_code | 0.000655 | 0.50% | 20.2021 | 5.2627 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 65 | 65 | 0 | 0 | 100.00% | 0.00% |
| q2 | 64 | 64 | 0 | 0 | 100.00% | 0.00% |
| q3 | 64 | 64 | 0 | 0 | 100.00% | 0.00% |
| q4 | 65 | 65 | 0 | 0 | 100.00% | 0.00% |
