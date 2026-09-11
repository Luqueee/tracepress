# Tracepress Baseline n20

Metadata-only report. No request/response payloads, identifiers, fingerprints, headers, or prices are included.

## Manifest

- `tracepress_commit`: 70957bba
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
- Requests: 39.
- Request kinds: `{"turn": 39}`.

## Measurement quality

- Analysis: 100.00% (39/39).
- Correlation: 100.00% (39/39).
- Forwarding errors: 0.
- Context malformed: 0.
- Token-estimation coverage by blocks: 65.54%.
- Semantic coverage mean: 94.16%.
- Provider observation partial: 19.

## Provider usage

- Input: 854,568; cached: 537,856; uncached: 316,712.
- Output: 30,349; reasoning: 26,009.
- Cache ratio: 62.94%.
- Reconciliation available: `False`; unavailable reasons: `{"missing_local_estimate": 39}`.

## Token-weighted composition

| Category | Estimated tokens | Token share | Blocks |
| --- | ---: | ---: | ---: |
| plain_text | 415,573 | 47.37% | 170 |
| json | 231,352 | 26.37% | 19 |
| unknown | 226,570 | 25.83% | 451 |
| source_code | 3,789 | 0.43% | 42 |

## Repetition and exposure

- Exact repeated blocks: 285; semantic: 190.
- Exact repeated token share: 35.66%; semantic: 35.66%.
- Stable explicit-prefix estimate: 0; share: 0.00%.
- Unknown detected content: 226,570 estimated tokens (25.83%).

## Compaction

- Cohort: `naturalistic`; requests: 0; V2: 0; legacy: 0.
- Trigger seen: 0; output seen: 0.

## Phase 4 candidate priority

This is a ranking signal, not an expected savings percentage.

| Rank | Category | Score | Token share | Redundancy | Persistence |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain_text | 0.126518 | 47.37% | 3.8938 | 1.8085 |
| 2 | json | 0.014697 | 26.37% | unknown | 1 |
| 3 | unknown | 0.005569 | 25.83% | 3.9160 | 1.7787 |
| 4 | source_code | 0.000600 | 0.43% | 4.2325 | 1.8837 |

## Limitations

- token estimates are heuristic.
- Subscription has no API-cost semantics.
- provider-managed context may be partially invisible.
- candidate exposure is not saveable tokens.
- cost is intentionally unavailable.
