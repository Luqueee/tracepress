# TRACEPRESS_CACHE_KEY_ATTRIBUTION_019

Phase 6.6 metadata-only cache-key attribution Shadow study.

Decision: **blocked_by_observable_prefix_difference**.

## Arms

| Arm | Runs | Requests | Input | Cached | Uncached |
|---|---:|---:|---:|---:|---:|
| `hooks_implicit_enabled` | 6 | 12 | 135199 | 90368 | 44831 |
| `hooks_explicit_enabled` | 6 | 12 | 135111 | 113664 | 21447 |
| `hooks_explicit_disabled` | 6 | 12 | 134571 | 110592 | 23979 |

## Observable-prefix gate

Developer maps / request metadata / complete initial block manifests match: **True/False/False**.
Observable prefix equivalent: **False**.
Request metadata mismatch rounds: **{"request_bytes": 4}**.

| Divergent initial block | Role | Origin | Rounds | Fields |
|---|---|---|---:|---|
| `/input/0` (`unknown`) | `developer` | `unknown` | 6 | `exact_fingerprint` |
| `/input/1` (`message`) | `developer` | `human_authored` | 6 | `exact_fingerprint` |
| `/input/2` (`message`) | `developer` | `human_authored` | 6 | `exact_fingerprint,raw_bytes` |
| `/input/3` (`message`) | `user` | `human_authored` | 6 | `exact_fingerprint,raw_bytes` |
| `/input/3/content/0/text` (`text`) | `user` | `human_authored` | 2 | `estimated_tokens,exact_fingerprint,raw_bytes` |
| `/input/3/content/1/text` (`text`) | `user` | `human_authored` | 6 | `exact_fingerprint` |
| `/input/4` (`message`) | `user` | `human_authored` | 6 | `exact_fingerprint,raw_bytes` |

## Cache attribution

Implicit vs explicit enabled uncached delta: **0.7056**.
Implicit enabled vs disabled uncached delta: **0.6061**.
Explicit enabled vs disabled uncached delta: **0.1115**.
Classification: **blocked_by_observable_prefix_difference**.

No cache-key value was observed or persisted. This study attributes only behavior visible through controlled configuration and aggregate provider usage.

Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, fingerprint, cache-key value, configuration content, authentication material, session id, or request id is in this report.
