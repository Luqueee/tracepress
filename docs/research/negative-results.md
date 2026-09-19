# Tracepress negative-results registry

This registry records measured candidates that are not active policies. Entries are metadata-only
and intentionally do not include prompts, paths, tool arguments, raw results, or candidate bytes.

| Candidate | Scope | Result | Decision |
|---|---|---|---|
| `json.minify` | ToolResult JSON | No material real improvement | Control only |
| `json.readable_table` | Homogeneous ToolResult JSON | No material applicability in tested cohorts | Closed for active use |
| `json.compact_records` | Homogeneous ToolResult JSON | No material applicability in tested cohorts | Closed for active use |
| `json.key_elision` | Homogeneous ToolResult JSON | No material applicability in tested cohorts | Closed for active use |
| `json.empty_noise_fields` | ToolResult JSON | 0 applicable blocks in the naturalistic reducers cohorts | Shadow only |
| `json.repeated_value_elision` | ToolResult JSON | 2.49% addressable share and 0.68% estimated reduction in the final pilot | Below materiality gate |
| `json.tabular` | ToolResult JSON | Small structural upper bound; opaque custom representation | Shadow-only upper bound |
| `search.result_projection` over provider-native envelope | Controlled public Search workload | Native envelopes were below the material reduction threshold | Closed for current candidates |
| `toolresult.lifetime` historical eviction | Public multi-request shadow cohort | 3.04% historical exposure; best net total-context reduction 2.76%, below Phase 4.6 gates | Closed without recovery or active eviction |
| `source.passthrough` Codex hook wrapper | Fully isolated `cargo test` N=10 A/A | Hook/proxy instrumentation was complete, but passthrough changed provider input materially (433,889 to 308,562) with no reducer enabled | Reject causal interpretation; require byte-faithful control evidence before Shadow |
| `source.path_shim` | Codex PATH interception smoke | Agent exited before provider, hook, or source execution; no sandbox bypass attempted | Closed for this runtime |
| `source.explicit_passthrough` | Pinned public `cargo test` N=10 paired A/A | 20/20 successful sessions, byte-identical source output, paired median request delta 0 | Accepted as the Phase 5.0 causal control surface |
| `cargo_test_v1` Shadow | Pinned public `cargo test` N=10 paired Shadow | Candidate output -93.99% with raw bytes still forwarded; provider deltas are therefore non-causal noise | Accept candidate for Phase 5.1; do not claim provider savings |
| `cargo_test_v1_active` | Pinned public `cargo test` N=10 paired active pilot | Source -93.52%, uncached input -35.77%, task success 10/10 in both arms, no retries or recalls | Accepted for this workload behind explicit opt-in; default remains passthrough |
| `cargo_check_v1` Shadow | Pinned public `cargo check` N=10 paired Shadow | Candidate -94.76%, raw bytes still forwarded, task success 10/10, no retries | Accept candidate for a separate active pilot; provider deltas are non-causal A/A noise |
| `cargo_check_v1_active` | Pinned public `cargo check` active smoke N=1 paired | Source -89.52%, but recovery 1/1 after final `Finished` status was removed | Reject v1 active; runtime selection blocked by the Phase 5.9 policy registry |
| `cargo_check_v2` Shadow | Pinned public `cargo check` N=10 paired Shadow | Candidate -89.98%, task success 10/10, no retries, raw still forwarded | Passed prerequisite for isolated v2 active pilot |
| `cargo_check_v2_active` | Success and diagnostic cohorts, each N=10 paired | Success source -84.74%, uncached -6.57%, no retries/recalls; diagnostics fail-open raw with one Treatment rerun | Accepted for this workload behind explicit opt-in; total input/output/reasoning/duration tradeoffs retained |
| `cargo_clippy_v1` Shadow | Pinned public `cargo clippy` paired smoke | Candidate -0.99%; 134,972 of 136,328 bytes remained because safe diagnostics dominated output | Reject active pilot and stop before N=10; do not add lossy grouping/dedup without a new evidence gate |
| `rg_v1` ambiguous Shadow smoke | Pinned public `rg -n fn crates` paired smoke | Four of 2,981 lines had multiple numeric-colon boundaries; whole stream returned raw-equivalent | Correct fail-closed behavior; retain as an active safety workload |
| `rg_v1` parseable Shadow | Pinned public `rg -n struct crates` N=10 paired | Lossless grouping candidate -31.19%, task success 10/10, no retries, raw still forwarded | Accept candidate for a separate active pilot; provider deltas are non-causal A/A noise |
| `rg_v1_active` ambiguous safety cohort | Pinned public ambiguous-output workload N=10 paired | 10/10 whole-stream fail-open, byte-exact raw emission, no mutations, retries, or recoveries | Accept safety behavior; do not attribute provider A/A deltas to the reducer |
| `rg_v1_active` parseable cohort | Pinned public parseable workload N=10 paired | Source -30.65%, uncached input -23.93% aggregate, paired median -8.5, task success 10/10, no retries or recalls | Accepted for this workload behind explicit opt-in; default remains passthrough |
| `git_status_v1` Shadow | Pinned public dirty-worktree workload N=10 paired | Candidate -55.19%, task success 10/10, no retries/recoveries, raw still forwarded | Accept candidate for a separate active pilot; provider deltas are non-causal A/A noise |
| `git_status_v1_active` | Dirty and clean cohorts, each N=10 paired | Dirty source -23.84%, but uncached input +3.32% aggregate despite paired median -9.5; clean 10/10 raw fail-open; no retries/recalls | Reject active policy for this workload; runtime selection blocked by the Phase 5.9 policy registry |
| `tool-surface-shadow-pilot-014` | Pinned public Search workload, N=10 | 20/20 complete request analyses and 10 observed tool calls, but the intercepted provider surface exposed 0 tool definitions, 0 schema bytes, and 0 estimated schema tokens | Reject active tool selection on this surface; do not build a selector without a materially different integration or workload |
| `instruction.workspace_marker_v1` attribution | Pinned public Search workload with ignored user config | Controlled root `AGENTS.md` marker produced 0 stable structural token delta in both smoke and N=3 attribution arms | Do not use this mode as proof of workspace-instruction attribution; explicit developer marker is the calibrated control |
| `instruction.hooks_disabled` decomposition | Pinned public Search workload, 3 interleaved rounds | Disabling Codex hooks produced 0 removed and 0 added stable instruction tokens/request | Do not treat hooks as part of the observed stable user-config instruction delta for this workload |
| Phase 6.4 provider cache controls | Adjacent A/A plus structurally identical `no_hooks` control across 3 interleaved rounds | Adjacent uncached A/A passed at 0.31%, but the null control differed by 139.02% uncached despite only 0.08% total-input delta | Reject provider cache-savings attribution from this cohort; require a fully position-balanced crossover baseline |
| Phase 6.5 cache-split crossover | Fully task/position/carryover-balanced control vs structurally identical `no_hooks`, 18 sessions | Total input matched within 0.06%, but uncached differed 120.78% aggregate and 116–127% in every position | Cache split is configuration-sensitive beyond the observed stable instruction map; block cached/uncached treatment claims pending cache-key attribution |
| Phase 6.6 cache-key attribution | Implicit hooks, explicit enable, and explicit disable in a fully balanced 18-session crossover | Stable developer maps matched, but the complete initial prefix differed in 6/6 rounds; explicit enable/disable uncached distance was 11.15% and did not reproduce Phase 6.5's separation | Reject feature-state and override-presence attribution from independent sessions; require ephemeral cache-key equality and dynamic-prefix observation |
| Phase 6.7 cache-key equality | One-process equality observer across the fully balanced 18-session hooks cohort | 36/36 requests carried a key; both requests within each session matched, but all 18 sessions used distinct equality classes and no matched cross-arm pair shared a class | Independent sessions are not cache-key-equivalent controls; stop cache-split causality pending an explicitly active normalization design |

The evidence is workload-scoped. It closes generic deterministic/provider-readable expansion for
the current cohorts; it does not claim that every future tool-family policy is impossible.

Phase 4.4 therefore selects a family from metadata-only exposure before implementing a reducer.
