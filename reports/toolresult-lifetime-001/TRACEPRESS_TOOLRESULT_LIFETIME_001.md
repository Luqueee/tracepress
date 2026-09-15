# TRACEPRESS_TOOLRESULT_LIFETIME_001

## Scope

Offline, metadata-only ToolResult lifetime analysis. No forwarding mutation, recovery object, or raw ToolResult content is produced.

## Dataset

- Snapshots: 50
- Context blocks: 1131
- ToolResult lineages: 40
- Estimated total context: 2113379

## Historical exposure

- Retained token exposure: 64198
- Historical ToolResult share: 0.030376946113309537

## Offline gate

- Lineages with a later request available: 30
- Best policy: `E1`
- Best shadow net total-context reduction: 0.027613598885954674
- Decision: `close_phase_4_6_without_runtime_eviction`

## Limitations

- Local token estimates are heuristic, not provider usage.
- Cache risk is a structural prefix proxy, not provider cache behavior.
- Lineages without call association or exact fingerprints fail closed and are excluded.
- No forwarding request was read or modified.

## Lifetime distributions

| Content kind | Tool family | Workload | Lineages | P50 | P95 | P99 | Max |
|---|---|---|---:|---:|---:|---:|---:|
| `all` | `all` | `public_lifetime_chain` | 40 | 1.5 | 3.0 | 3.0 | 3 |
| `all` | `exec` | `all` | 40 | 1.5 | 3.0 | 3.0 | 3 |
| `json` | `all` | `all` | 40 | 1.5 | 3.0 | 3.0 | 3 |
| `json` | `exec` | `all` | 40 | 1.5 | 3.0 | 3.0 | 3 |

## Age buckets

| Age | Estimated exposure | Share of total context |
|---:|---:|---:|
| >= 1 | 64198 | 0.030376946113309537 |
| >= 2 | 39404 | 0.01864502297032383 |
| >= 3 | 15345 | 0.007260884110232949 |
| >= 5 | 0 | 0.0 |
| >= 10 | 0 | 0.0 |

## Size x age

| Size | Age | ToolResults | Estimated exposure | Share of total context |
|---|---:|---:|---:|---:|
| `<256` | 1 | 28 | 1899 | 0.0008985610247854265 |
| `<256` | 2 | 11 | 714 | 0.0003378475890978381 |
| `<256` | 3 | 4 | 264 | 0.000124918436305083 |
| `<256` | 5 | 0 | 0 | 0.0 |
| `<256` | 10+ | 0 | 0 | 0.0 |
| `256-1K` | 1 | 8 | 7136 | 0.003376583187398001 |
| `256-1K` | 2 | 4 | 3568 | 0.0016882915936990005 |
| `256-1K` | 3 | 0 | 0 | 0.0 |
| `256-1K` | 5 | 0 | 0 | 0.0 |
| `256-1K` | 10+ | 0 | 0 | 0.0 |
| `1K-4K` | 1 | 24 | 55163 | 0.026101801901126112 |
| `1K-4K` | 2 | 15 | 35122 | 0.01661888378752699 |
| `1K-4K` | 3 | 6 | 15081 | 0.007135965673927866 |
| `1K-4K` | 5 | 0 | 0 | 0.0 |
| `1K-4K` | 10+ | 0 | 0 | 0.0 |
| `4K-16K` | 1 | 0 | 0 | 0.0 |
| `4K-16K` | 2 | 0 | 0 | 0.0 |
| `4K-16K` | 3 | 0 | 0 | 0.0 |
| `4K-16K` | 5 | 0 | 0 | 0.0 |
| `4K-16K` | 10+ | 0 | 0 | 0.0 |
| `>16K` | 1 | 0 | 0 | 0.0 |
| `>16K` | 2 | 0 | 0 | 0.0 |
| `>16K` | 3 | 0 | 0 | 0.0 |
| `>16K` | 5 | 0 | 0 | 0.0 |
| `>16K` | 10+ | 0 | 0 | 0.0 |

## Stub and recovery schema overhead

- Stub: 94 bytes; 24 locally estimated tokens.
- Recovery schema: 351 bytes; 88 locally estimated tokens.

## Policy simulations

| Policy | Eligible historical ToolResults | Gross reduction | Stub overhead | Recovery schema overhead | Shadow net reduction | Net total-context reduction | Cache-risk distribution |
|---|---:|---:|---:|---:|---:|---:|---|
| `E0` | 0 | 0 | 0 | 0 | 0 | 0.0 | `{}` |
| `E1` | 60 | 64198 | 1440 | 4400 | 58358 | 0.027613598885954674 | `{'Medium': 60}` |
| `E2` | 30 | 39404 | 720 | 4400 | 34284 | 0.01622236238743737 | `{'Medium': 30}` |
| `E3` | 10 | 15345 | 240 | 4400 | 10705 | 0.005065347956992096 | `{'Medium': 10}` |
| `E4` | 19 | 38690 | 456 | 3080 | 35154 | 0.01663402541617003 | `{'Medium': 19}` |
| `E5` | 6 | 21342 | 144 | 1320 | 19878 | 0.009405790442698635 | `{'Medium': 6}` |

## Cache-risk proxy and compaction

- Cache proxy is structural only: `{'Low': 'preserved prefix >= 90%', 'Medium': '50%-<90%', 'High': '<50%', 'Unknown': 'no raw offset'}`.
- Compaction snapshots observed: 0; available: false.
- A compaction boundary is a measured request-kind marker, not an assumed safe cache reset.
