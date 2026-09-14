# Phase 4.2.1 — Provider-Compatible Candidate Discovery

This phase separates structural compressibility from transformations a normal provider/model can
read directly. It does not rewrite forwarding traffic and does not claim provider token or cost
savings.

Only `ToolGenerated + ToolResult + Json` and `ToolGenerated + ToolResult + PlainText` are eligible.
`Unknown` is always `NeverTransform`; instructions, human/agent history, schemas, reasoning, and
provider-managed state are excluded.

| Candidate | Target | Provider readability | Recovery |
| --- | --- | --- | --- |
| `json.readable_table` | homogeneous `array<object>` | human-readable structured | SHA-256 exact |
| `json.compact_records` | homogeneous `array<object>` | human-readable structured | SHA-256 exact |
| `json.key_elision` | homogeneous `array<object>` | human-readable structured | SHA-256 exact |
| `text.readable_line_fold` | exact repeated lines | human-readable structured | SHA-256 exact |
| `text.readable_block_fold` | exact repeated blocks | human-readable structured | SHA-256 exact |
| `text.log_prefix_fold` | conservative structured logs | human-readable structured | SHA-256 exact |

Nested JSON values remain compact JSON cells. No lossy summarization, omission, top-K, or opaque
binary encoding is used. `json.minify` is a lossless control formally marked
`rejected_no_real_improvement` after the provider-compatible diagnostic. `json.tabular`/TPJ2 remains
shadow-only as an opaque decoder-dependent upper bound.

Shadow persistence records provider readability and bounded shape labels alongside each candidate:
JSON root kind, array/key-count buckets, homogeneity, primitive/nested cell ratios, and plain-text
shape. Candidate bodies are never persisted. `scripts/characterize_provider_shapes.py` reads a
database in SQLite read-only mode and produces a metadata-only distribution for up to ten sessions.

The Observatory Compression Lab exposes addressable estimated-token share, applicability, effective
reduction, recovery/determinism, latency, structural cache-risk evidence, and provider readability.
`cache_risk` remains structural evidence, not a provider-cache prediction.

Naturalistic Shadow Pilot 003 completed 10 sessions and 31 provider requests. It recorded 0
forwarding mutations, 0 recovery failures, 0 determinism failures, and 0 Unknown transformations,
but 24 bounded shadow drops. The human-readable candidates observed 0% addressable/effective
reduction in this cohort; `json.minify` remained a no-improvement control. Phase 4.3 is therefore
blocked and no active candidate is selected. The current conclusion is an insufficient
deterministic/reversible opportunity for active compression; any lossy or selective strategy needs
a separately designed quality phase.
