# TRACEPRESS_PROVIDER_CACHE_CROSSOVER_018

Phase 6.5 fully position-balanced provider-cache crossover. Shadow only.

Decision: **crossover_baseline_failed_provider_reliability**.

## Arms

| Arm | Runs | Requests | Stable tokens/request | Input | Cached | Uncached |
|---|---:|---:|---:|---:|---:|---:|
| `user_config_control` | 6 | 12 | 21445 | 280613 | 265216 | 15397 |
| `no_hooks_null` | 6 | 12 | 21445 | 280458 | 218112 | 62346 |
| `no_extensions_treatment` | 6 | 12 | 7301 | 136025 | 113664 | 22361 |

## Structural crossover

Control/null exact stable-map match: **True**.
Treatment removed/added tokens per request: **15684/1540**.

## Cache controls

Null total-input relative delta: **0.0006** (threshold 0.02).
Null aggregate uncached relative delta: **1.2078** (threshold 0.1).
Null per-position uncached deltas: **0=1.2740, 1=1.1614, 2=1.1902** (max threshold 0.2).
Total-input comparison reliable: **True**. Cache-split comparison reliable: **False**.
Provider comparison reliable: **False**.

## Treatment observation

Input/cached/uncached relative changes: **-0.5153/-0.5714/0.4523**.
Output/reasoning relative changes: **0.0953/0.1431**.
Interpretation: **blocked_by_cache_controls**. Task quality was not evaluated, so no policy is authorized.

Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, identity, configuration content, authentication material, session id, or request id is in this report.
