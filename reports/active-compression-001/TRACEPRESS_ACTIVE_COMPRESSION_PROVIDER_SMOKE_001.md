# TRACEPRESS ACTIVE COMPRESSION PROVIDER SMOKE 001

Configuration smoke against the authenticated ChatGPT Codex subscription path. This run was
performed in an isolated temporary Tracepress home with `gpt-5.6-luna` and Codex's sandbox forced
to `read-only`.

| Arm | Provider requests | Content encoding | Active attempts | Rewrites | Analysis |
|---|---:|---|---:|---:|---:|
| Control (`off`) | 2 | `zstd` | 0 | 0 | 2/2 complete |
| Active (`json.minify`) | 3 | `zstd` | 0 | 0 | 3/3 complete |

The active adapter intentionally accepts only identity-encoded bodies. Because Codex sent all
requests with `Content-Encoding: zstd`, the active arm failed open to the original bytes and did
not attempt a rewrite. This is expected behavior, not evidence that the candidate was evaluated
against the provider.

The provider path itself was reachable and both arms completed. The Codex client also probed the
proxy's `/v1/models` route (which is not part of the Tracepress proxy contract) and logged a
non-fatal 404; this should be resolved or explicitly disabled before a production-quality A/B.

No provider-token, cache, cost, or quality conclusion can be drawn from this smoke. Supporting
active rewriting for this transport requires a separately designed bounded zstd
decode/edit/re-encode path, which is outside Phase 4.1.
