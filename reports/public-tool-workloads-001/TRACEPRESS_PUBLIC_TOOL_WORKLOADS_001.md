# TRACEPRESS_PUBLIC_TOOL_WORKLOADS_001

Status: **completed** · shadow-only · metadata-only aggregation.

Cohort: **6 Search + 6 Tests = 12 bounded tasks**. Forwarding mutations: **0**. Shadow drops: **0**.

## Pinned public repositories

- `BurntSushi/ripgrep` at `3fce3b5bb0236da2df6d99672afb8a719642eca7`
- `pytest-dev/pytest` at `de30d8417f3d93bc0c16fc0a64092e197c46e36a`

## Family results

| Family | ToolResults | Estimated tokens | Exposure | P50 bytes | P95 bytes | Candidate | Applicable | Effective reduction | Canonical correctness | Active eligible |
|---|---:|---:|---:|---:|---:|---|---:|---:|---|---|
| `search` | 6 | 18371 | 68.94% | 1354 | 18774 | `search.result_projection` | 6 | 29.74% | True | True |
| `tests` | 6 | 8276 | 31.06% | 3827 | 8154 | `—` | 0 | 0.00% | — | False |

## Decision

Search projection is implemented and semantically checked, but this bounded public command cohort is not an active A/B. Tests are characterization-only. Provider usage, cache effects, and task quality remain unclaimed.
