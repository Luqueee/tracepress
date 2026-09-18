# TRACEPRESS_USER_CONFIG_LAYER_DECOMPOSITION_017

Phase 6.4 controlled public cache-aware layer decomposition. Shadow only.

Decision: **structural_decomposition_complete_policy_blocked**.

## Arms

| Arm | Runs | Requests | Stable tokens/request | Input | Cached | Uncached |
|---|---:|---:|---:|---:|---:|---:|
| `user_config_a` | 3 | 6 | 21445 | 140338 | 103936 | 36402 |
| `user_config_b` | 3 | 6 | 21445 | 140224 | 103936 | 36288 |
| `no_plugins` | 3 | 6 | 13012 | 91494 | 65024 | 26470 |
| `no_memories` | 3 | 6 | 15734 | 117129 | 75264 | 41865 |
| `no_hooks` | 3 | 6 | 21445 | 140169 | 133632 | 6537 |
| `no_extensions` | 3 | 6 | 7301 | 68503 | 39680 | 28823 |
| `ignore_user_config` | 3 | 6 | 7301 | 78092 | 62976 | 15116 |

## Layer attribution

| Metric | Tokens/request |
|---|---:|
| `common_all_arms_tokens_per_request` | 5761 |
| `plugins_removed_tokens_per_request` | 9973 |
| `plugins_added_tokens_per_request` | 1540 |
| `memories_removed_tokens_per_request` | 5711 |
| `memories_added_tokens_per_request` | 0 |
| `hooks_removed_tokens_per_request` | 0 |
| `hooks_added_tokens_per_request` | 0 |
| `combined_removed_tokens_per_request` | 15684 |
| `combined_added_tokens_per_request` | 1540 |
| `combined_removal_explained_by_independent_layers_tokens_per_request` | 15684 |
| `combined_interaction_only_removed_tokens_per_request` | 0 |
| `configured_no_extensions_only_tokens_per_request` | 0 |
| `ignored_config_only_tokens_per_request` | 0 |

## Cache A/A

Structural A-only/B-only tokens per request: **0/0**.

Uncached-input relative delta: **0.0031**. Threshold: **0.1**. Within threshold: **True**.

Structurally null `no_hooks` uncached/input relative deltas: **1.3902/0.0008**. Uncached within threshold: **False**.

Provider usage remains descriptive and is not attributed savings.

Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, identity, configuration content, authentication material, session id, or request id is in this report.
