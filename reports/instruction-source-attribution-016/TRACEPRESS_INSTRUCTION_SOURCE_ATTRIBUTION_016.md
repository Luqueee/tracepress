# TRACEPRESS_INSTRUCTION_SOURCE_ATTRIBUTION_016

Phase 6.3 controlled public instruction-source attribution. Shadow only; no ordinary provider request or instruction policy was modified.

Runtime: **codex-cli 0.155.0**. Model: **gpt-5.6-luna**. Sessions per arm: **3**.

Decision: **attribution_calibrated_policy_still_blocked**.

## Arms

| Arm | Requests | Complete | Stable tokens/request | Input | Cached | Uncached |
|---|---:|---:|---:|---:|---:|---:|
| `user_config` | 6 | 6 | 21510 | 144260 | 139776 | 4484 |
| `ignore_user_config` | 6 | 6 | 7301 | 81939 | 66048 | 15891 |
| `ignore_user_config_workspace_marker` | 6 | 6 | 7301 | 82126 | 51968 | 30158 |
| `ignore_user_config_developer_marker` | 6 | 6 | 7358 | 82094 | 65024 | 17070 |

## Structural attribution

| Metric | Estimated tokens/request |
|---|---:|
| Stable blocks common to all arms | 5761 |
| User-config-only stable blocks | 15749 |
| Ignored-config-only stable blocks | 1540 |
| Workspace-marker-only stable blocks | 0 |
| Stable blocks removed/replaced by workspace marker | 0 |
| Developer-marker-only stable blocks | 57 |
| Stable blocks removed/replaced by developer marker | 0 |

Provider usage is observed separately and is not attributed savings.

Privacy: aggregate allowlist only; no instruction text, prompt, command, path, response, fingerprint, semantic path, configuration content, authentication material, session id, or request id is in this report.
