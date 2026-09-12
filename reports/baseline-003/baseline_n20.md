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
- Requests: 84.
- Request kinds: `{"turn": 84}`.

## Measurement quality

- Analysis: 100.00% (84/84).
- Request ledger: 84 eligible; 84 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (84/84).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 64.40%.
- Semantic coverage mean: 95.23%.
- Provider observation partial: 64.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | plain_text | 421/421 (100.00%) | 3,030,888/3,030,888 (100.00%) | 43.41% |
| detected_content_kind | json | 136/136 (100.00%) | 1,970,142/1,970,142 (100.00%) | 33.80% |
| detected_content_kind | unknown | 538/1204 (44.68%) | 1,449,707/8,896,780 (16.29%) | 22.24% |
| detected_content_kind | source_code | 110/110 (100.00%) | 36,650/36,650 (100.00%) | 0.54% |
| context_block_kind | text | 933/933 (100.00%) | 4,422,489/4,422,489 (100.00%) | 65.17% |
| context_block_kind | tool_result | 136/136 (100.00%) | 1,970,142/1,970,142 (100.00%) | 33.80% |
| context_block_kind | tool_call | 136/136 (100.00%) | 94,756/94,756 (100.00%) | 1.03% |
| context_block_kind | message | 0/429 (0.00%) | 0/4,574,772 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/153 (0.00%) | 0/513,665 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/84 (0.00%) | 0/2,358,636 (0.00%) | 0.00% |
| context_origin | human_authored | 840/1176 (71.43%) | 4,397,949/8,918,066 (49.32%) | 64.83% |
| context_origin | tool_generated | 136/136 (100.00%) | 1,970,142/1,970,142 (100.00%) | 33.80% |
| context_origin | agent_generated | 229/322 (71.12%) | 119,296/173,951 (68.58%) | 1.37% |
| context_origin | provider_managed | 0/153 (0.00%) | 0/513,665 (0.00%) | 0.00% |
| context_origin | unknown | 0/84 (0.00%) | 0/2,358,636 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 20.
- Admitted: 84; deferred: 0; processed: 84.
- High-water items P50/P90/P99: 1/1/1.
- High-water bytes P50/P90/P99: 54810/60894.1000/66826.9000.
- Analysis wait µs P50/P90/P99: 22/28.1000/30.6200.
- Scheduler field coverage: high-water items 100.00%; high-water bytes 100.00%; wait 100.00%.
- Runtime counter consistency: `True`; reported seen: 84; ledger eligible: 84; delta: 0.

## Scheduler sidecar integrity

- Gate: `passed`; eligible requests: 84; complete snapshots: 84.
- Sidecar sessions: 20/20; missing: 0; capture issues: 0; counter mismatches: 0.
- Reported analysis seen: 84; unexplained counter delta: 0.
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

## Provider usage

- Input: 2,093,562; cached: 1,582,336; uncached: 511,226.
- Output: 63,967; reasoning: 46,618.
- Cache ratio: 75.58%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 84}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| plain_text | 968,477 | 43.41% | 421 |
| json | 754,052 | 33.80% | 136 |
| unknown | 496,196 | 22.24% | 1204 |
| source_code | 12,029 | 0.54% | 110 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| human_authored | text | plain_text | 958,742 | 42.98% | 2,995,217 |
| tool_generated | tool_result | json | 754,052 | 33.80% | 1,970,142 |
| human_authored | text | unknown | 480,388 | 21.53% | 1,385,764 |
| agent_generated | tool_call | unknown | 12,723 | 0.57% | 54,056 |
| human_authored | text | source_code | 6,972 | 0.31% | 16,968 |
| agent_generated | tool_call | plain_text | 5,231 | 0.23% | 21,018 |
| agent_generated | tool_call | source_code | 5,057 | 0.23% | 19,682 |
| agent_generated | text | plain_text | 4,504 | 0.20% | 14,653 |
| agent_generated | text | unknown | 3,085 | 0.14% | 9,887 |
| agent_generated | message | unknown | 0 | 0.00% | 54,655 |
| human_authored | message | unknown | 0 | 0.00% | 4,520,117 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 513,665 |
| unknown | unknown | unknown | 0 | 0.00% | 2,358,636 |

## Composition by workload

Workload labels are harness metadata; token shares use the estimable-token subset.

| Workload | Sessions | Session share | Estimated tokens | Workload token share | Bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| bug_diagnosis | 2 | 10.00% | 168,803 | 7.57% | 1,220,925 |
| code_review | 2 | 10.00% | 235,959 | 10.58% | 1,400,907 |
| compaction_investigation | 3 | 15.00% | 302,954 | 13.58% | 2,041,461 |
| dependency_investigation | 2 | 10.00% | 228,497 | 10.24% | 1,338,232 |
| feature_implementation | 2 | 10.00% | 188,800 | 8.46% | 1,256,994 |
| large_search | 3 | 15.00% | 365,890 | 16.40% | 2,224,736 |
| refactor | 2 | 10.00% | 243,504 | 10.92% | 1,401,637 |
| repo_exploration | 2 | 10.00% | 217,444 | 9.75% | 1,446,451 |
| test_debugging | 2 | 10.00% | 278,903 | 12.50% | 1,603,117 |

### Session-weighted detected content

Each session contributes one normalized composition before averaging.

| Detected kind | Mean session token share | Sessions with estimates |
| --- | ---: | ---: |
| json | 31.31% | 20 |
| plain_text | 45.09% | 20 |
| source_code | 0.57% | 20 |
| unknown | 23.03% | 20 |

### Workload x detected content

| Workload | Detected kind | Estimated tokens | Share within workload | Bytes |
| --- | --- | ---: | ---: | ---: |
| bug_diagnosis | plain_text | 92,377 | 54.72% | 289,200 |
| bug_diagnosis | unknown | 46,649 | 27.64% | 849,411 |
| bug_diagnosis | json | 28,386 | 16.82% | 78,077 |
| bug_diagnosis | source_code | 1,391 | 0.82% | 4,237 |
| code_review | json | 95,160 | 40.33% | 256,092 |
| code_review | plain_text | 92,281 | 39.11% | 289,015 |
| code_review | unknown | 46,520 | 19.72% | 849,251 |
| code_review | source_code | 1,998 | 0.85% | 6,549 |
| compaction_investigation | plain_text | 150,739 | 49.76% | 472,510 |
| compaction_investigation | unknown | 75,195 | 24.82% | 1,366,280 |
| compaction_investigation | json | 75,037 | 24.77% | 196,171 |
| compaction_investigation | source_code | 1,983 | 0.65% | 6,500 |
| dependency_investigation | plain_text | 92,389 | 40.43% | 289,117 |
| dependency_investigation | json | 88,755 | 38.84% | 202,300 |
| dependency_investigation | unknown | 46,689 | 20.43% | 845,199 |
| dependency_investigation | source_code | 664 | 0.29% | 1,616 |
| feature_implementation | plain_text | 91,531 | 48.48% | 286,027 |
| feature_implementation | json | 48,472 | 25.67% | 127,448 |
| feature_implementation | unknown | 47,268 | 25.04% | 838,020 |
| feature_implementation | source_code | 1,529 | 0.81% | 5,499 |
| large_search | plain_text | 149,228 | 40.78% | 466,420 |
| large_search | json | 137,497 | 37.58% | 366,524 |
| large_search | unknown | 77,309 | 21.13% | 1,386,475 |
| large_search | source_code | 1,856 | 0.51% | 5,317 |
| refactor | json | 101,810 | 41.81% | 259,660 |
| refactor | plain_text | 92,502 | 37.99% | 289,717 |
| refactor | unknown | 48,339 | 19.85% | 849,889 |
| refactor | source_code | 853 | 0.35% | 2,371 |
| repo_exploration | plain_text | 104,178 | 47.91% | 326,126 |
| repo_exploration | json | 59,295 | 27.27% | 162,714 |
| repo_exploration | unknown | 52,963 | 24.36% | 954,868 |
| repo_exploration | source_code | 1,008 | 0.46% | 2,743 |
| test_debugging | json | 119,640 | 42.90% | 321,156 |
| test_debugging | plain_text | 103,252 | 37.02% | 322,756 |
| test_debugging | unknown | 55,264 | 19.81% | 957,387 |
| test_debugging | source_code | 747 | 0.27% | 1,818 |

## Repetition and exposure

- Exact repeated blocks: 1,283; semantic: 692.
- Exact repeated token share: 66.85%; semantic: 49.57%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 496,196 estimated tokens (22.24%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| human_authored | text | plain_text | 42.98% | 730,471 | 730,471 | 4/5 |
| tool_generated | tool_result | json | 33.80% | 373,793 | 0 | 2/3 |
| human_authored | text | unknown | 21.53% | 366,007 | 366,007 | 4/5 |
| agent_generated | tool_call | unknown | 0.57% | 6,514 | 0 | 2/3 |
| human_authored | text | source_code | 0.31% | 5,312 | 5,312 | 4/5 |
| agent_generated | tool_call | plain_text | 0.23% | 2,800 | 0 | 3/3 |
| agent_generated | tool_call | source_code | 0.23% | 2,336 | 0 | 2/3 |
| agent_generated | text | plain_text | 0.20% | 2,527 | 2,527 | 2.5000/4 |
| agent_generated | text | unknown | 0.14% | 1,566 | 1,566 | 2/3 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 2/3 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 4/5 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 2/3 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 4/5 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[90565.75, 105707.5, 134265.5]`.
- Context-size quartile boundaries: `[18192.0, 23312.0, 33175.5]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 8, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 8, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 8, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 8, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "compaction_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 8, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 8, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 8, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 8, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 13, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 13, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 8, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 8, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 9, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 9, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 9, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 9, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 84, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 84, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.095627 | 43.41% | 8.3897 | 3.6787 |
| 2 | json | 0.007532 | 33.80% | unknown | 2.1250 |
| 3 | unknown | 0.003713 | 22.24% | 8.4527 | 3.3898 |
| 4 | source_code | 0.000727 | 0.54% | 11.4464 | 3.5926 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 21 | 21 | 0 | 0 | 100.00% | 0.00% |
| q2 | 21 | 21 | 0 | 0 | 100.00% | 0.00% |
| q3 | 21 | 21 | 0 | 0 | 100.00% | 0.00% |
| q4 | 21 | 21 | 0 | 0 | 100.00% | 0.00% |
