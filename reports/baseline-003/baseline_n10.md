# Tracepress Baseline n10

Metadata-only report. No request/response payloads, raw fingerprints, headers, or prices are included; the ledger contains opaque storage identities for accounting.

## Manifest

- `tracepress_commit`: a15ac2dafa5074e447b8dd2811165f8f4be30c9f
- `measurement_tooling_commit`: 3bd135344feced756a299f9dd7eac415e4c4d68f
- `measurement_instrument_version`: 2
- `runtime_instrument_version`: 2
- `sidecar_schema_version`: 2
- `convergence_gate_version`: 2
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
