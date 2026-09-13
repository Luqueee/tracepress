# TRACEPRESS ACTIVE COMPRESSION PROVIDER SMOKE 003

Directed authenticated Codex smoke after separating decoded-candidate metrics from zstd wire
metrics. The prompt asked Codex to emit one large heterogeneous JSON tool result. State and logs
were isolated from the operational daemon; no request content is persisted in this report.

| Arm | Provider requests | Content encoding | Active attempts | Rewrites | No improvement | Not applicable | Analysis |
|---|---:|---|---:|---:|---:|---:|---:|
| Control (`off`) | 2 | `zstd` | 0 | 0 | — | — | 2/2 complete |
| Active (`json.minify`) | 2 | `zstd` | 2 | 0 | 1 | 1 | 2/2 complete |

The active arm entered the bounded zstd decode/edit/re-encode path. One request had no eligible
target and one eligible attempt did not produce a smaller decoded candidate for this provider
payload. Because no rewrite was accepted, the provider received the original wire bytes in both
arms. Resource-limit, invalid-input, recovery, determinism, internal-error, and analysis-drop
counters were all zero.

The active adapter now applies the never-worse guard to the decoded representation and records
`wire_input_bytes`, `wire_output_bytes`, and `wire_bytes_delta` independently. A wire-size change
is transport evidence only; it is not provider token, cache, cost, or quality evidence.

Both arms reached the provider and completed context analysis. The Codex client still probes the
unsupported `/v1/models` route and logs a non-fatal 404; that remains outside this experiment.

Runtime commit: `17608d27235e31ef4b5d7a5ea01023a97c55437a`
