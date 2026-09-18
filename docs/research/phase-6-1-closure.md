# Phase 6.1 closure

Phase 6.1 closes as a valid negative Shadow result with no provider or tool mutation.

| Closure item | Result |
|---|---|
| Public pinned workload | BurntSushi/ripgrep at `3fce3b5bb0236da2df6d99672afb8a719642eca7` |
| Codex runtime | `codex-cli 0.155.0` |
| Sessions complete and successful | 10/10 |
| Provider requests analyzed | 20/20 complete |
| Schema observation coverage | 100% |
| Tool calls observed | 10 |
| Tool definitions/schema bytes/schema tokens | 0 / 0 / 0 |
| Known-zero metric semantics | Corrected and regression-tested |
| Aggregate-only privacy boundary | Preserved |
| Provider or tool mutation | None |
| Active tool-selection pilot | Rejected for this surface |

The complete cohort provides sufficient evidence for a negative decision: Codex used a tool in each
session, but the provider requests visible to Tracepress did not carry its definitions. There is no
measured schema surface for an active selector to reduce at this interception point.

Further work must not add selector breadth here. A new attempt requires a materially different
integration or workload and must first demonstrate observable tool-definition exposure in Shadow.
