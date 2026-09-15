# TRACEPRESS_PUBLIC_TOOLRESULT_LIFETIME_SOURCE_001

Bounded public metadata-only characterization of provider-native Search ToolResult shapes.

Sessions: **1**. Provider requests: **5**. Context snapshots complete: **5/5**.
Lifetime chain: **true**.
Search-projection target observed: **true**.

| Origin | Block kind | Role | Detected content kind | Blocks | Raw bytes | Estimated tokens |
|---|---|---|---|---:|---:|---:|
| `human_authored` | `text` | `developer` | `plain_text` | 15 | 179815 | 57650 |
| `human_authored` | `text` | `developer` | `unknown` | 15 | 63925 | 21850 |
| `human_authored` | `text` | `user` | `unknown` | 15 | 20955 | 7465 |
| `agent_generated` | `tool_call` | `tool` | `unknown` | 10 | 5847 | 1388 |
| `tool_generated` | `tool_result` | `tool` | `json` | 10 | 3881 | 616 |
| `human_authored` | `text` | `user` | `source_code` | 5 | 1010 | 415 |
| `agent_generated` | `text` | `assistant` | `plain_text` | 4 | 408 | 128 |
| `human_authored` | `message` | `developer` | `unavailable` | 10 | 247485 | — |
| `human_authored` | `message` | `user` | `unavailable` | 10 | 25490 | — |
| `provider_managed` | `opaque_reasoning` | `assistant` | `unavailable` | 10 | 26458 | — |
| `unknown` | `unknown` | `developer` | `unavailable` | 5 | 154405 | — |
| `agent_generated` | `message` | `assistant` | `unavailable` | 4 | 1700 | — |

Privacy: aggregate allowlist only; no prompt, command, path, source, ToolResult, response, or fingerprint is in this report.
