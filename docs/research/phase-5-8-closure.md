# Phase 5.8 closure

Phase 5.8 closes with a rejected active policy and a passing clean-output safety contract.

| Closure item | Result |
|---|---|
| Dirty active cohort | Rejected, N=10 paired |
| Clean fail-open cohort | Passed, N=10 paired |
| Source reduction | 23.84% |
| Aggregate uncached input | 56,136 to 58,000 (+3.32%) |
| Paired median uncached delta | -9.5 |
| Objective task quality | 10/10 per arm in both cohorts |
| Requests / tool calls | No increase |
| Retries / recoveries | 0 / 0 |
| Clean forwarding mutations | 0 |
| Default runtime policy | Passthrough |

`git_status_v1_active` is not accepted because provider uncached input failed the positive gate. No
broader or lossier Git status reducer is authorized from this result. A future retry requires new
evidence, a materially different workload, or a reduced recovery-overhead design—not periodic
reruns of the same cohort. Phase 5.9 subsequently blocks this reducer from CLI runtime selection.
