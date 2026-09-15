# TRACEPRESS_PUBLIC_TOOLRESULT_LIFETIME_SOURCE_001

Bounded public metadata-only characterization of provider-native Search ToolResult shapes.

Sessions: **10**. Provider requests: **50**. Context snapshots complete: **50/50**.
Lifetime chain: **true**.
Search-projection target observed: **true**.

| Origin | Block kind | Role | Detected content kind | Blocks | Raw bytes | Estimated tokens |
|---|---|---|---|---:|---:|---:|
| `human_authored` | `text` | `developer` | `plain_text` | 150 | 1798150 | 576500 |
| `human_authored` | `text` | `developer` | `unknown` | 150 | 639250 | 218500 |
| `tool_generated` | `tool_result` | `tool` | `json` | 100 | 234348 | 89421 |
| `human_authored` | `text` | `user` | `unknown` | 150 | 209550 | 74650 |
| `agent_generated` | `tool_call` | `tool` | `unknown` | 100 | 57311 | 12672 |
| `human_authored` | `text` | `user` | `source_code` | 50 | 10100 | 4150 |
| `agent_generated` | `text` | `assistant` | `plain_text` | 40 | 4580 | 1368 |
| `provider_managed` | `opaque_reasoning` | `assistant` | `unavailable` | 101 | 252385 | — |
| `human_authored` | `message` | `developer` | `unavailable` | 100 | 2474825 | — |
| `human_authored` | `message` | `user` | `unavailable` | 100 | 254935 | — |
| `unknown` | `unknown` | `developer` | `unavailable` | 50 | 1544050 | — |
| `agent_generated` | `message` | `assistant` | `unavailable` | 40 | 17528 | — |

Privacy: aggregate allowlist only; no prompt, command, path, source, ToolResult, response, or fingerprint is in this report.
