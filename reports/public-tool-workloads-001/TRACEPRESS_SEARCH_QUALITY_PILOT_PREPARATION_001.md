# public-search-quality-pilot-001

Status: **prepared, not started**. This is a metadata-only preparation artifact; no active Control/Treatment session ran.

Reducer: `search.result_projection` v1. Assignment unit: **session**. Planned pairs: **6**.

## Pinned public workload

- `BurntSushi/ripgrep` at `3fce3b5bb0236da2df6d99672afb8a719642eca7`
- Commands, arguments, paths, source, and ToolResult bytes were execution-only.

## Objective answer keys

| Task | Order | Exit | Matches | Unique files | Evaluator |
|---|---|---:|---:|---:|---|
| `search-01` | `control_first` | 0 | 20 | 7 | `KnownAnswer` |
| `search-02` | `treatment_first` | 0 | 547 | 56 | `KnownAnswer` |
| `search-03` | `treatment_first` | 0 | 13 | 10 | `KnownAnswer` |
| `search-04` | `control_first` | 0 | 321 | 57 | `KnownAnswer` |
| `search-05` | `control_first` | 0 | 38 | 13 | `KnownAnswer` |
| `search-06` | `treatment_first` | 0 | 2 | 1 | `KnownAnswer` |

## Pilot gate

The Search shadow candidate passed the Phase 4.5 opportunity gate, so this manifest prepares a future paired quality pilot. It does not authorize or execute active request rewriting.

Before starting, the runner must persist session-level Control/Treatment assignments, keep treatment fail-open, collect objective outcomes, and compare provider cached/uncached input. No economic or universal quality claim follows from this preparation artifact.

Privacy audit: **no raw content persisted**. Forwarding mutations: **0**. Active sessions: **0**.
