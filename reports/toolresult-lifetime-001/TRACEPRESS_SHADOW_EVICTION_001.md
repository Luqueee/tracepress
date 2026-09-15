# TRACEPRESS_SHADOW_EVICTION_001

Shadow-only candidate evaluation. Provider forwarding remains unchanged; no recovery object or payload replacement is performed.

Candidates: **9**. Forwarding mutations: **0**. Candidates dropped by bound: **0**.

| Policy | Candidates | Gross est. tokens | Stub est. tokens | Schema overhead | Net shadow reduction | Cache risk |
|---|---:|---:|---:|---:|---:|---|
| `E1` | 6 | 381 | 144 | 9944 | -9707 | `{'Medium': 6}` |
| `E2` | 3 | 186 | 72 | 9944 | -9830 | `{'Medium': 3}` |
| `E5` | 0 | 0 | 0 | 0 | 0 | `{}` |

Privacy: opaque lineage and occurrence identifiers plus bounded metadata only; no raw ToolResult content, content hash, CAS path, or recovery ID is emitted.
