# Phase 5.6 closure

Phase 5.6 closes with an accepted, explicitly opt-in `rg_v1_active` policy for the measured public
workload and a passing ambiguous-output fail-open cohort.

| Closure item | Result |
|---|---|
| Parseable active cohort | Passed, N=10 paired |
| Ambiguous safety cohort | Passed, N=10 paired |
| Source reduction | 30.65% |
| Aggregate uncached input | 135,502 to 103,079 (-23.93%) |
| Paired median uncached delta | -8.5 |
| Objective task quality | 10/10 per arm in both cohorts |
| Requests / tool calls | No increase |
| Retries / recoveries | 0 / 0 |
| Ambiguous forwarding mutations | 0 |
| Never-worse hint accounting | Complete |
| Observatory preferred report | `rg_v1_active` parseable cohort |
| Default runtime policy | Passthrough |

No capping, truncation, deduplication, pipeline rewriting, or breadth expansion was added. Phase 5.6
does not by itself authorize `git status`; any next reducer must begin as a separate Shadow phase.
