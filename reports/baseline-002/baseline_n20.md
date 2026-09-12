# Tracepress Baseline n20

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

- Sessions: 20 total; 20 closed.
- Requests: 189.
- Request kinds: `{"compaction_v2": 1, "turn": 188}`.

## Measurement quality

- Analysis: 100.00% (189/189).
- Request ledger: 189 eligible; 189 complete; 0 partial; 0 dropped.
- Measurement integrity: `passed`; conflicts: 0; unmatched drop events: 0.
- Auxiliary drops: 0 events / 0 work units.
- Correlation: 100.00% (189/189).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 63.84%.
- Semantic coverage mean: 96.92%.
- Provider observation partial: 142.
- Analysis deferral rate: 0.00%.
- Analysis loss rate: 0.00%.
- Backlog capacity drops: 0.

## Estimation coverage by category

The composition token shares use the estimable-token subset; these tables show coverage bias by category.

| Dimension | Category | Blocks estimated/total | Bytes estimated/total | Estimated token share |
| --- | --- | ---: | ---: | ---: |
| detected_content_kind | json | 1856/1856 (100.00%) | 42,005,864/42,005,864 (100.00%) | 82.28% |
| detected_content_kind | plain_text | 849/849 (100.00%) | 6,770,029/6,770,029 (100.00%) | 10.62% |
| detected_content_kind | unknown | 2763/6105 (45.26%) | 4,329,148/25,263,253 (17.14%) | 6.71% |
| detected_content_kind | source_code | 433/433 (100.00%) | 264,101/264,101 (100.00%) | 0.39% |
| context_block_kind | tool_result | 1857/1857 (100.00%) | 42,006,191/42,006,191 (100.00%) | 82.28% |
| context_block_kind | text | 2187/2187 (100.00%) | 10,007,736/10,007,736 (100.00%) | 16.11% |
| context_block_kind | tool_call | 1857/1857 (100.00%) | 1,355,215/1,355,215 (100.00%) | 1.62% |
| context_block_kind | message | 0/1053 (0.00%) | 0/10,378,688 (0.00%) | 0.00% |
| context_block_kind | opaque_reasoning | 0/2098 (0.00%) | 0/5,148,770 (0.00%) | 0.00% |
| context_block_kind | unknown | 0/191 (0.00%) | 0/5,406,647 (0.00%) | 0.00% |
| context_origin | tool_generated | 1857/1857 (100.00%) | 42,006,191/42,006,191 (100.00%) | 82.28% |
| context_origin | human_authored | 1890/2646 (71.43%) | 9,905,115/20,084,968 (49.32%) | 15.95% |
| context_origin | agent_generated | 2154/2451 (87.88%) | 1,457,836/1,656,671 (88.00%) | 1.77% |
| context_origin | provider_managed | 0/2098 (0.00%) | 0/5,148,770 (0.00%) | 0.00% |
| context_origin | unknown | 0/191 (0.00%) | 0/5,406,647 (0.00%) | 0.00% |

## Deferred-analysis scheduler

- Runtime sidecar available: `True`; sessions with metrics: 20.
- Admitted: 190; deferred: 0; processed: 190.
- High-water items P50/P90/P99: 1/2/2.
- High-water bytes P50/P90/P99: 45026/349298.6000/507533.9600.
- Analysis wait µs P50/P90/P99: 149/1278.2000/1618.5200.
- Scheduler field coverage: high-water items 85.00%; high-water bytes 85.00%; wait 85.00%.
- Runtime counter consistency: `False`; reported seen: 190; ledger eligible: 189; delta: 1.

## Scheduler sidecar integrity

- Gate: `failed`; eligible requests: 189; complete snapshots: 189.
- Sidecar sessions: 20/20; missing: 0; capture issues: 20; counter mismatches: 3.
- Reported analysis seen: 190; unexplained counter delta: 1.
- Duplicate sidecar IDs: `[]`; unexpected IDs: `[]`; malformed records: 0.

| Session | Measurement run | PID | Sidecar file | Eligible | Complete snapshots | Admitted | Processed + drops | Status | Capture classification | Missing identity | Missing capture fields |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| 01a095aa-95bc-7070-8c48-adfddceafc9b | None | unknown | None | 18 | 18 | 18 | 18 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095aa-95c5-7081-862c-8f8a48c662f3 | None | unknown | None | 24 | 24 | 24 | 24 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095ad-6eff-70a3-ba33-5b5b0afec3ed | None | unknown | None | 36 | 36 | 36 | 36 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095ad-6f06-7bf0-a433-802ed60247b5 | None | unknown | None | 17 | 17 | 17 | 17 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095b3-4e70-7a70-bddf-112614d88b4c | None | unknown | None | 33 | 33 | 33 | 33 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095b3-4e79-75c3-b152-e34981926b47 | None | unknown | None | 16 | 16 | 16 | 16 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095c5-ac8f-7111-9197-c342bf4f735c | None | unknown | None | 3 | 3 | 3 | 3 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095c5-ac95-7331-955b-ff4ff16ef356 | None | unknown | None | 2 | 2 | 2 | 2 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095c6-4a5d-7ef0-9148-4b525137db99 | None | unknown | None | 7 | 7 | 7 | 7 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095c6-4a63-7ac1-ad79-fe5a13eb454f | None | unknown | None | 3 | 3 | 3 | 3 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095e8-d6a4-7391-924c-d3f4e165f9aa | None | unknown | None | 6 | 6 | 6 | 6 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `["high_water_items", "high_water_bytes", "analysis_wait_us"]` |
| 01a095e8-d6a4-7391-924c-d40d27981f16 | None | unknown | None | 6 | 6 | 6 | 6 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095ea-1401-7143-8519-a18564099145 | None | unknown | None | 4 | 4 | 4 | 4 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `["high_water_items", "high_water_bytes", "analysis_wait_us"]` |
| 01a095ea-1407-73d2-935e-f78f70067a54 | None | unknown | None | 4 | 4 | 4 | 4 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `["high_water_items", "high_water_bytes", "analysis_wait_us"]` |
| 01a095eb-4152-7691-8107-46d26ae926cb | None | unknown | None | 1 | 1 | 1 | 1 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095eb-4159-7102-aee7-3a95df59bc6b | None | unknown | None | 1 | 1 | 3 | 3 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095eb-790f-72c1-8205-bc0618a12a57 | None | unknown | None | 3 | 3 | 3 | 3 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095eb-7914-7cd2-8def-4d37a2135dc2 | None | unknown | None | 1 | 1 | 1 | 1 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095eb-d6f6-7250-b3e4-395c636ac709 | None | unknown | None | 3 | 3 | 1 | 1 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |
| 01a095eb-d6fe-74f3-85c5-38fa391fa625 | None | unknown | None | 1 | 1 | 2 | 2 | legacy_uncertified | legacy_uncertified | `["measurement_run_id", "tracepress_pid", "capture_complete"]` | `[]` |

## Provider usage

- Input: 13,478,969; cached: 12,433,152; uncached: 1,045,817.
- Output: 67,184; reasoning: 35,296.
- Cache ratio: 92.24%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 189}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| json | 16,780,209 | 82.28% | 1856 |
| plain_text | 2,165,930 | 10.62% | 849 |
| unknown | 1,369,289 | 6.71% | 6105 |
| source_code | 79,216 | 0.39% | 433 |

## Context composition cross-tab

| Origin | Block kind | Detected kind | Estimated tokens | Token share | Bytes |
| --- | --- | --- | ---: | ---: | ---: |
| tool_generated | tool_result | json | 16,780,209 | 82.28% | 42,005,864 |
| human_authored | text | plain_text | 2,156,692 | 10.57% | 6,738,756 |
| human_authored | text | unknown | 1,081,518 | 5.30% | 3,128,181 |
| agent_generated | tool_call | unknown | 266,118 | 1.30% | 1,129,292 |
| agent_generated | tool_call | source_code | 63,529 | 0.31% | 225,923 |
| agent_generated | text | unknown | 21,653 | 0.11% | 71,675 |
| human_authored | text | source_code | 15,687 | 0.08% | 38,178 |
| agent_generated | text | plain_text | 9,221 | 0.05% | 30,946 |
| tool_generated | tool_result | plain_text | 17 | 0.00% | 327 |
| agent_generated | message | unknown | 0 | 0.00% | 198,835 |
| human_authored | message | unknown | 0 | 0.00% | 10,179,853 |
| provider_managed | opaque_reasoning | unknown | 0 | 0.00% | 5,148,770 |
| unknown | unknown | unknown | 0 | 0.00% | 5,406,647 |

## Repetition and exposure

- Exact repeated blocks: 8,363; semantic: 1,962.
- Exact repeated token share: 91.99%; semantic: 14.41%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 1,369,289 estimated tokens (6.71%).

## Repetition by structural category

| Origin | Block kind | Detected kind | Token share | Exact repeated tokens | Semantic repeated tokens | Persistence P50/P90 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| tool_generated | tool_result | json | 82.28% | 15,522,787 | 0 | 9/24.4000 |
| human_authored | text | plain_text | 10.57% | 1,928,492 | 1,928,492 | 4/24 |
| human_authored | text | unknown | 5.30% | 967,462 | 967,462 | 4/33 |
| agent_generated | tool_call | unknown | 1.30% | 242,767 | 0 | 8.5000/24 |
| agent_generated | tool_call | source_code | 0.31% | 57,675 | 0 | 12.5000/29.2000 |
| agent_generated | text | unknown | 0.11% | 20,300 | 20,300 | 10/31 |
| human_authored | text | source_code | 0.08% | 14,027 | 14,027 | 4/24.9000 |
| agent_generated | text | plain_text | 0.05% | 8,312 | 8,312 | 5/30 |
| tool_generated | tool_result | plain_text | 0.00% | 0 | 0 | 1/1 |
| agent_generated | message | unknown | 0.00% | 0 | 0 | 6/30.6000 |
| human_authored | message | unknown | 0.00% | 0 | 0 | 4/24 |
| provider_managed | opaque_reasoning | unknown | 0.00% | 0 | 0 | 9/24 |
| unknown | unknown | unknown | 0.00% | 0 | 0 | 4/24 |

## Missingness

Drop rate is based on exclusive request outcomes, never on event count.
- Request-size quartile boundaries: `[132726.0, 252489.0, 478204.0]`.
- Context-size quartile boundaries: `[28448.0, 81355.0, 158238.0]`.
- Unavailable dimensions: `[]`.
- Workload strata: `[{"complete_requests": 43, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 43, "name": "bug_diagnosis", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 17, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 17, "name": "bug_fix", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 1, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 1, "name": "code_review", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 3, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 3, "name": "compaction_calibration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "dependency_investigation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 37, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 37, "name": "feature_implementation", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 33, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 33, "name": "large_search", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 4, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 4, "name": "refactor", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 24, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 24, "name": "repo_exploration", "partial_requests": 0, "request_coverage": 1.0}, {"complete_requests": 23, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 23, "name": "test_debugging", "partial_requests": 0, "request_coverage": 1.0}]`.
- Concurrency strata: `[{"complete_requests": 189, "drop_rate": 0.0, "dropped_requests": 0, "eligible_requests": 189, "name": "concurrent_pair", "partial_requests": 0, "request_coverage": 1.0}]`.

## Compaction

- Cohort: `naturalistic`; requests: 1; V2: 1; legacy: 0.
- Trigger seen: 1; output seen: 1.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.008747 | 10.62% | 18.9060 | 9.1290 |
| 2 | json | 0.006412 | 82.28% | unknown | 11.1138 |
| 3 | unknown | 0.000585 | 6.71% | 21.4235 | 10.4388 |
| 4 | source_code | 0.000522 | 0.39% | 57.1705 | 10.7241 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.

## Missingness strata

| Request-size stratum | Eligible | Complete | Partial | Dropped | Coverage | Drop rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| q1 | 48 | 48 | 0 | 0 | 100.00% | 0.00% |
| q2 | 47 | 47 | 0 | 0 | 100.00% | 0.00% |
| q3 | 47 | 47 | 0 | 0 | 100.00% | 0.00% |
| q4 | 47 | 47 | 0 | 0 | 100.00% | 0.00% |
