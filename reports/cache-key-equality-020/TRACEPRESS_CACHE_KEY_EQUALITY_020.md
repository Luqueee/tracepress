# TRACEPRESS_CACHE_KEY_EQUALITY_020

Phase 6.7 single-process ephemeral cache-key equality Shadow study.

Decision: **session_scoped_cache_key_classes**.

Runs/requests/provider requests: **18/36/36**.
Cache-key status counts: **{"present": 36}**.
Unique ephemeral equality classes: **18**.
Requests per run: **{"2": 18}**.
Same class across all requests within every run: **True**.

## Cross-arm equality

| Matched arm pair | Equal first-request classes |
|---|---:|
| `hooks_implicit_enabled__hooks_explicit_enabled` | 0/6 |
| `hooks_implicit_enabled__hooks_explicit_disabled` | 0/6 |
| `hooks_explicit_enabled__hooks_explicit_disabled` | 0/6 |

## Provider usage

| Arm | Input | Cached | Uncached | Output | Reasoning |
|---|---:|---:|---:|---:|---:|
| `hooks_implicit_enabled` | 159373 | 119808 | 39565 | 2539 | 1632 |
| `hooks_explicit_enabled` | 158971 | 144384 | 14587 | 2343 | 1449 |
| `hooks_explicit_disabled` | 159125 | 135168 | 23957 | 2188 | 1214 |

Raw cache-key values existed only in process memory. The report contains ordinal equality classes only as aggregate counts; it contains no key value or reusable hash.
