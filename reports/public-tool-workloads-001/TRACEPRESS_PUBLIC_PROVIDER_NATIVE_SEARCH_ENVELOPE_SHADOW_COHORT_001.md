# TRACEPRESS_PUBLIC_PROVIDER_NATIVE_SEARCH_SHAPE_001

Bounded public metadata-only characterization of provider-native Search ToolResult shapes.

Sessions: **10**. Provider requests: **20**. Context snapshots complete: **20/20**.
Search-projection target observed: **true**.

| Origin | Block kind | Role | Detected content kind | Blocks | Raw bytes | Estimated tokens |
|---|---|---|---|---:|---:|---:|
| `human_authored` | `text` | `developer` | `plain_text` | 60 | 719260 | 230600 |
| `human_authored` | `text` | `developer` | `unknown` | 60 | 255700 | 87400 |
| `human_authored` | `text` | `user` | `unknown` | 40 | 74980 | 27280 |
| `human_authored` | `text` | `user` | `source_code` | 20 | 4040 | 1660 |
| `agent_generated` | `tool_call` | `tool` | `unknown` | 10 | 6655 | 1632 |
| `human_authored` | `text` | `user` | `plain_text` | 20 | 4394 | 1278 |
| `tool_generated` | `tool_result` | `tool` | `json` | 10 | 3896 | 641 |
| `agent_generated` | `text` | `assistant` | `plain_text` | 7 | 701 | 209 |
| `human_authored` | `message` | `developer` | `unavailable` | 40 | 989938 | — |
| `human_authored` | `message` | `user` | `unavailable` | 40 | 97520 | — |
| `unknown` | `unknown` | `developer` | `unavailable` | 20 | 617620 | — |
| `provider_managed` | `opaque_reasoning` | `assistant` | `unavailable` | 10 | 35478 | — |
| `agent_generated` | `message` | `assistant` | `unavailable` | 7 | 2968 | — |

| Candidate | Status | Evaluations | Input bytes | Output bytes | Byte reduction | Recovery verified | Deterministic |
|---|---|---:|---:|---:|---:|---:|---:|
| `json.compact_records` | `no_improvement` | 10 | 1260 | 1430 | — | 10 | 10 |
| `json.empty_noise_fields` | `no_improvement` | 1 | 154 | — | — | 0 | 1 |
| `json.key_elision` | `no_improvement` | 10 | 1260 | 1470 | — | 10 | 10 |
| `json.minify` | `no_improvement` | 10 | 1260 | 1260 | 0 | 10 | 10 |
| `json.noop` | `no_improvement` | 10 | 1260 | 1260 | 0 | 10 | 10 |
| `json.readable_table` | `no_improvement` | 10 | 1260 | 1480 | — | 10 | 10 |
| `json.repeated_subtree` | `not_applicable` | 10 | 1260 | — | — | 0 | 0 |
| `json.repeated_value_elision` | `no_improvement` | 1 | 154 | — | — | 0 | 1 |
| `json.tabular` | `no_improvement` | 10 | 1260 | 1230 | 30 | 10 | 10 |

Privacy: aggregate allowlist only; no prompt, command, path, source, ToolResult, response, or fingerprint is in this report.
