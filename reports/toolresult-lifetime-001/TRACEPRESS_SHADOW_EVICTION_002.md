# TRACEPRESS_SHADOW_EVICTION_001

Shadow-only candidate evaluation. Provider forwarding remains unchanged; no recovery object or payload replacement is performed.

Candidates: **96**. Forwarding mutations: **0**. Candidates dropped by bound: **0**.

| Policy | Candidates | Gross est. tokens | Stub est. tokens | Schema overhead | Net shadow reduction | Cache risk |
|---|---:|---:|---:|---:|---:|---|
| `E1` | 60 | 64198 | 1440 | 4400 | 58358 | `{'Medium': 60}` |
| `E2` | 30 | 39404 | 720 | 4400 | 34284 | `{'Medium': 30}` |
| `E5` | 6 | 21342 | 144 | 1320 | 19878 | `{'Medium': 6}` |

Privacy: opaque lineage and occurrence identifiers plus bounded metadata only; no raw ToolResult content, content hash, CAS path, or recovery ID is emitted.
