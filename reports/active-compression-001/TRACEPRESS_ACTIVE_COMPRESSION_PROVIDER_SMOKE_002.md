# TRACEPRESS ACTIVE COMPRESSION PROVIDER SMOKE 002

Infrastructure smoke against the authenticated ChatGPT Codex subscription path after adding the
bounded zstd decode/edit/re-encode arm. The run used isolated Tracepress state, `gpt-5.6-luna`,
Codex sandbox `read-only`, and the fixed subscription endpoint.

| Arm | Provider requests | Content encoding | Active attempts | Rewrites | No improvement | Not applicable | Analysis |
|---|---:|---|---:|---:|---:|---:|---:|
| Control (`off`) | 2 | `zstd` | 0 | 0 | — | — | 2/2 complete |
| Active (`json.minify`) | 2 | `zstd` | 2 | 0 | 1 | 1 | 2/2 complete |

The active arm entered the zstd path and decoded both requests under the bounded decoder. One
request had no eligible target span; the other reached the candidate but the re-encoded wire body
was not smaller, so the never-worse guard classified it as `NoImprovement` and forwarded the
original wire bytes. There were zero resource-limit, invalid-input, recovery, determinism, or
internal failures.

Both arms reached the provider and completed their context analysis. This verifies transport
reachability and the fail-open/forwarding boundary; it is not a provider A/B efficacy result.

The Codex client still probes `/v1/models`, which Tracepress does not expose, and logs a non-fatal
404. That endpoint is outside this compression change and should be resolved or explicitly
disabled before a production-quality A/B.

No provider-token, cache, cost, quality, or causal savings conclusion can be drawn. The provider
receives zstd-compressed request bytes, while this smoke only validates bounded local
decode/edit/re-encode behavior and active-arm accounting.

Runtime commit: `fb8ae07452072a18973a329edcb642b652ecc9e1`
