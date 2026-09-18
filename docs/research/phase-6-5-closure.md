# Phase 6.5 closure

Phase 6.5 closes as a partial measurement success and a valid cache-split negative.

| Closure item | Result |
|---|---|
| Controlled arms | 3 |
| Williams rounds | 6 across 2 task blocks |
| Sessions | 18/18 successful and ephemeral |
| Provider snapshots | 36/36 complete |
| Position balance | Every arm × task × position exactly once |
| Directed carryover balance | Every ordered arm pair twice |
| Control/null structural match | Exact at 21,445 tokens/request |
| Null total-input delta | 0.06%; passed 2% gate |
| Null aggregate uncached delta | 120.78%; failed 10% gate |
| Null per-position uncached deltas | 127.40%, 116.14%, 119.02%; all failed 20% gate |
| Treatment total-input observation | -51.53%; quality not evaluated |
| Aggregate-only privacy boundary | Preserved |
| Active instruction policy | Blocked |

The next eligible phase is cache-key attribution in Shadow mode. It must explain or bound the
control/null cache divergence without persisting provider content before any uncached-input claim or
active instruction policy is considered.
