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
- Requests: 142.
- Request kinds: `{"turn": 142}`.

## Measurement quality

- Analysis: 100.00% (142/142).
- Request ledger: 142 eligible; 142 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (142/142).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 64.49%.
- Semantic coverage mean: 95.41%.
- Provider observation partial: 112.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | plain_text | 722/722 (100.00%) | 5,131,882/5,131,882 (100.00%) | 39.98% |
| detected_content_kind | json | 278/278 (100.00%) | 4,138,366/4,138,366 (100.00%) | 38.80% |
| detected_content_kind | unknown | 954/2137 (44.64%) | 2,480,995/15,213,239 (16.31%) | 20.62% |
| detected_content_kind | source_code | 194/194 (100.00%) | 74,991/74,991 (100.00%) | 0.60% |
| context_block_kind | text | 1592/1592 (100.00%) | 7,482,572/7,482,572 (100.00%) | 59.96% |
| context_block_kind | tool_result | 278/278 (100.00%) | 4,138,366/4,138,366 (100.00%) | 38.80% |
| context_block_kind | tool_call | 278/278 (100.00%) | 205,296/205,296 (100.00%) | 1.25% |
| context_block_kind | message | 0/740 (0.00%) | 0/7,744,769 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/301 (0.00%) | 0/1,000,257 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/142 (0.00%) | 0/3,987,218 (0.00%) | 0.00% |
| context_origin | human_authored | 1420/1988 (71.43%) | 7,436,129/15,078,744 (49.32%) | 59.61% |
| context_origin | tool_generated | 278/278 (100.00%) | 4,138,366/4,138,366 (100.00%) | 38.80% |
| context_origin | agent_generated | 450/622 (72.35%) | 251,739/353,893 (71.13%) | 1.59% |
| context_origin | provider_managed | 0/301 (0.00%) | 0/1,000,257 (0.00%) | 0.00% |
| context_origin | unknown | 0/142 (0.00%) | 0/3,987,218 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 30.
- Admitted: 142; deferred: 0; processed: 142.
- High-water items P50/P90/P99: 1/1/1.
- High-water bytes P50/P90/P99: 56893.5000/68418.9000/89261.3800.
- Analysis wait µs P50/P90/P99: 24.5000/40.1000/46.6800.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 142; ledger eligible: 142; delta: 0.

## Scheduler sidecar integrity

- Gate: `passed`; eligible requests: 142; complete snapshots: 142.
- Sidecar sessions: 30/30; missing: 0; capture issues: 0; counter mismatches: 0.
- Reported analysis seen: 142; unexplained counter delta: 0.
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
| 01a09676-3a33-7780-9b83-83182ab34fc7 | f3d29534-69ca-423f-be9d-f8ed3517676e | 1,972,480 | f3d29534-69ca-423f-be9d-f8ed3517676e.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a09676-3a40-7641-90e6-0e363063de5c | f10382e2-7c86-434f-b7fd-59fe07a139d6 | 1,972,519 | f10382e2-7c86-434f-b7fd-59fe07a139d6.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09677-7961-7b10-a15e-01fb9c0f0e78 | 8f7090af-a58d-471a-9ad4-34052816d07b | 1,978,262 | 8f7090af-a58d-471a-9ad4-34052816d07b.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09677-796f-7d91-876f-a73608b0d317 | 64df2b5a-0698-4b7d-9d51-907020b63b62 | 1,978,304 | 64df2b5a-0698-4b7d-9d51-907020b63b62.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09678-8aa1-79b3-8040-0435063a2379 | 69ac892e-87cb-40fd-a44d-0741d1fbe9f0 | 1,983,520 | 69ac892e-87cb-40fd-a44d-0741d1fbe9f0.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09678-8aab-7001-8fd8-3fd24691e98f | aee9371a-f180-45bc-adac-a740f1f33631 | 1,983,555 | aee9371a-f180-45bc-adac-a740f1f33631.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09679-b950-7ac2-9d44-3f31ff01efd8 | 3a691447-f414-4c89-aaef-4e0914a04728 | 1,989,147 | 3a691447-f414-4c89-aaef-4e0914a04728.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a09679-b95f-7051-94e0-9f88f793d96c | e9f32319-3bb9-498d-8efc-00d189e187c6 | 1,989,189 | e9f32319-3bb9-498d-8efc-00d189e187c6.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a0967b-2b39-7493-9e91-25980a588f96 | efd13857-47bd-4892-99ab-9511f58df07e | 1,995,440 | efd13857-47bd-4892-99ab-9511f58df07e.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a0967b-2b46-7ff3-a134-dd1be7a76871 | 8a63fb14-907c-4e9b-80ac-a228442ca05d | 1,995,481 | 8a63fb14-907c-4e9b-80ac-a228442ca05d.json | 4 | 4 | 4 | 4 | passed | complete | `[]` | `[]` |
| 01a096a4-c908-7091-b75b-8a7feabb2567 | 11c866f7-1d6f-412e-b772-2f44ee63f259 | 2,110,223 | 11c866f7-1d6f-412e-b772-2f44ee63f259.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a096a6-9643-78e1-9c8e-b1d2f63f7908 | 7915141c-99d7-40e4-9dea-41b65daaa932 | 2,117,653 | 7915141c-99d7-40e4-9dea-41b65daaa932.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a096a6-9652-7282-9132-f62430252381 | f79801d2-db1c-4342-9c93-d9086a5f0c14 | 2,117,694 | f79801d2-db1c-4342-9c93-d9086a5f0c14.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |
| 01a096a8-e030-7471-88bd-ca59ae0390d9 | db74bc89-17b2-41c5-a1db-b034a789f523 | 2,126,712 | db74bc89-17b2-41c5-a1db-b034a789f523.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a096a8-e03e-76e1-b03b-f12d386bdb56 | 6c0935f7-9b98-47c3-97ce-0a1cfe7b8f94 | 2,126,750 | 6c0935f7-9b98-47c3-97ce-0a1cfe7b8f94.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a096aa-fc47-71f2-af67-c7fb00e37efa | ce246702-142c-4c22-92be-a8fe4ba4f8c7 | 2,135,278 | ce246702-142c-4c22-92be-a8fe4ba4f8c7.json | 5 | 5 | 5 | 5 | passed | complete | `[]` | `[]` |
| 01a096aa-fc55-79e0-beb8-41a8782b8d9c | 4a159cc5-6f3e-4469-82b1-03323595c747 | 2,135,317 | 4a159cc5-6f3e-4469-82b1-03323595c747.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a096ad-7605-76f0-993f-e64a2afe7bc7 | 0677f0bd-da20-49e6-8fae-9502cd032ef8 | 2,144,876 | 0677f0bd-da20-49e6-8fae-9502cd032ef8.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a096ad-7612-7562-b1f4-6a68b3133251 | a935df03-f275-4e48-a6ab-30cdb14e8147 | 2,144,912 | a935df03-f275-4e48-a6ab-30cdb14e8147.json | 6 | 6 | 6 | 6 | passed | complete | `[]` | `[]` |
| 01a096af-d67e-7500-80f8-15f2a9eee913 | 5a87fd46-c473-47fb-89c5-070360fd79ee | 2,154,253 | 5a87fd46-c473-47fb-89c5-070360fd79ee.json | 7 | 7 | 7 | 7 | passed | complete | `[]` | `[]` |

## Provider usage

- Input: 3,746,507; cached: 2,862,848; uncached: 883,659.
- Output: 122,749; reasoning: 89,650.
- Cache ratio: 76.41%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 142}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| plain_text | 1,639,960 | 39.98% | 722 |
| json | 1,591,587 | 38.80% | 278 |
| unknown | 845,992 | 20.62% | 2137 |
| source_code | 24,558 | 0.60% | 194 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| human_authored | text | plain_text | 1,621,041 | 39.52% | 5,064,355 |
| tool_generated | tool_result | json | 1,591,587 | 38.80% | 4,138,366 |
| human_authored | text | unknown | 812,282 | 19.80% | 2,343,090 |
| agent_generated | tool_call | unknown | 27,460 | 0.67% | 117,861 |
| agent_generated | tool_call | source_code | 12,772 | 0.31% | 46,307 |
| human_authored | text | source_code | 11,786 | 0.29% | 28,684 |
| agent_generated | tool_call | plain_text | 10,844 | 0.26% | 41,128 |
| agent_generated | text | plain_text | 8,075 | 0.20% | 26,399 |
| agent_generated | text | unknown | 6,250 | 0.15% | 20,044 |
| agent_generated | message | unknown | 0 | 0.00% | 102,154 |
| human_authored | message | unknown | 0 | 0.00% | 7,642,615 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 1,000,257 |
| unknown | unknown | unknown | 0 | 0.00% | 3,987,218 |

## Composition by workload

Workload labels are harness metadata; token shares use the estimable-token subset.

| Workload | Sessions | Session share | Estimated tokens | Workload token share | Bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| bug_diagnosis | 4 | 13.33% | 419,042 | 10.22% | 2,860,046 |
| code_review | 3 | 10.00% | 428,103 | 10.44% | 2,502,328 |
| compaction_investigation | 3 | 10.00% | 302,954 | 7.39% | 2,041,461 |
| dependency_investigation | 4 | 13.33% | 531,015 | 12.94% | 3,187,971 |
| feature_implementation | 3 | 10.00% | 325,702 | 7.94% | 2,101,828 |
| large_search | 3 | 10.00% | 365,890 | 8.92% | 2,224,736 |
| long_running | 1 | 3.33% | 338,821 | 8.26% | 1,593,367 |
| refactor | 3 | 10.00% | 466,332 | 11.37% | 2,579,011 |
| repo_exploration | 3 | 10.00% | 421,435 | 10.27% | 2,576,812 |
| test_debugging | 3 | 10.00% | 502,803 | 12.26% | 2,890,918 |

### Session-weighted detected content

Each session contributes one normalized composition before averaging.

| Detected kind | Mean session token share | Sessions with estimates |
| --- | ---: | ---: |
| json | 34.58% | 30 |
| plain_text | 42.82% | 30 |
| source_code | 0.60% | 30 |
| unknown | 22.00% | 30 |

### Workload x detected content

| Workload | Detected kind | Estimated tokens | Share within workload | Bytes |
| --- | --- | ---: | ---: | ---: |
| bug_diagnosis | plain_text | 207,246 | 49.46% | 648,167 |
| bug_diagnosis | unknown | 107,478 | 25.65% | 1,933,820 |
| bug_diagnosis | json | 101,551 | 24.23% | 269,555 |
| bug_diagnosis | source_code | 2,767 | 0.66% | 8,504 |
| code_review | json | 180,844 | 42.24% | 494,231 |
| code_review | plain_text | 162,169 | 37.88% | 508,347 |
| code_review | unknown | 81,104 | 18.94% | 1,486,423 |
| code_review | source_code | 3,986 | 0.93% | 13,327 |
| compaction_investigation | plain_text | 150,739 | 49.76% | 472,510 |
| compaction_investigation | unknown | 75,195 | 24.82% | 1,366,280 |
| compaction_investigation | json | 75,037 | 24.77% | 196,171 |
| compaction_investigation | source_code | 1,983 | 0.65% | 6,500 |
| dependency_investigation | plain_text | 219,058 | 41.25% | 684,993 |
| dependency_investigation | json | 195,821 | 36.88% | 467,188 |
| dependency_investigation | unknown | 111,528 | 21.00% | 2,021,421 |
| dependency_investigation | source_code | 4,608 | 0.87% | 14,369 |
| feature_implementation | plain_text | 149,986 | 46.05% | 469,326 |
| feature_implementation | json | 96,940 | 29.76% | 252,069 |
| feature_implementation | unknown | 76,414 | 23.46% | 1,372,308 |
| feature_implementation | source_code | 2,362 | 0.73% | 8,125 |
| large_search | plain_text | 149,228 | 40.78% | 466,420 |
| large_search | json | 137,497 | 37.58% | 366,524 |
| large_search | unknown | 77,309 | 21.13% | 1,386,475 |
| large_search | source_code | 1,856 | 0.51% | 5,317 |
| long_running | json | 210,550 | 62.14% | 538,007 |
| long_running | plain_text | 83,715 | 24.71% | 262,602 |
| long_running | unknown | 41,745 | 12.32% | 784,679 |
| long_running | source_code | 2,811 | 0.83% | 8,079 |
| refactor | json | 217,060 | 46.55% | 557,713 |
| refactor | plain_text | 161,338 | 34.60% | 504,847 |
| refactor | unknown | 86,583 | 18.57% | 1,512,868 |
| refactor | source_code | 1,351 | 0.29% | 3,583 |
| repo_exploration | plain_text | 172,962 | 41.04% | 541,052 |
| repo_exploration | json | 156,320 | 37.09% | 421,835 |
| repo_exploration | unknown | 90,647 | 21.51% | 1,609,970 |
| repo_exploration | source_code | 1,506 | 0.36% | 3,955 |
| test_debugging | json | 219,967 | 43.75% | 575,073 |
| test_debugging | plain_text | 183,519 | 36.50% | 573,618 |
| test_debugging | unknown | 97,989 | 19.49% | 1,738,995 |
| test_debugging | source_code | 1,328 | 0.26% | 3,232 |

## Repetition and exposure

- Exact repeated blocks: 2,398; semantic: 1,228.
- Exact repeated token share: 68.37%; semantic: 47.23%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 845,992 estimated tokens (20.62%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| human_authored | text | plain_text | 39.52% | 1,278,577 | 1,278,577 | 4/6 |
| tool_generated | tool_result | json | 38.80% | 838,754 | 0 | 2/4 |
| human_authored | text | unknown | 19.80% | 640,680 | 640,680 | 4/6 |
| agent_generated | tool_call | unknown | 0.67% | 15,547 | 0 | 2/4 |
| agent_generated | tool_call | source_code | 0.31% | 6,438 | 0 | 2/3 |
| human_authored | text | source_code | 0.29% | 9,296 | 9,296 | 4/6 |
| agent_generated | tool_call | plain_text | 0.26% | 6,604 | 0 | 3/4.2000 |
| agent_generated | text | plain_text | 0.20% | 5,068 | 5,068 | 3/4.7000 |
| agent_generated | text | unknown | 0.15% | 3,697 | 3,697 | 3/4 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 3/4 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 4/6 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 2/4 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 4/6 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[91653.0, 107704.0, 140949.5]`.
- Context-size quartile boundaries: `[18452.25, 23584.0, 34512.0]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 18, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 18, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 14, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 14, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "compaction_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 19, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 19, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 7, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 7, "name": "long_running", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 14, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 14, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 15, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 15, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 16, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 16, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 135, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 135, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 7, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 7, "name": "single", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.078137 | 39.98% | 9.4627 | 4.1832 |
| 2 | json | 0.006987 | 38.80% | unknown | 2.4821 |
| 3 | unknown | 0.003021 | 20.62% | 9.5577 | 3.8018 |
| 4 | source_code | 0.000783 | 0.60% | 14.5960 | 3.9529 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 36 | 36 | 0 | 0 | 100.00% | 0.00% |
| q2 | 35 | 35 | 0 | 0 | 100.00% | 0.00% |
| q3 | 35 | 35 | 0 | 0 | 100.00% | 0.00% |
| q4 | 36 | 36 | 0 | 0 | 100.00% | 0.00% |
