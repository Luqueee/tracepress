# Phase 5.1 closure

Phase 5.1 closes with a positive, workload-scoped active result and no default policy change.

| Closure item | Result |
|---|---|
| Active `cargo_test_v1` | Implemented behind explicit environment opt-in |
| Never-worse includes final hint | Complete |
| Recovery before mutation | Complete |
| Random opaque recovery id | 128-bit token |
| Same-session and expiry enforcement | Complete |
| Missing-session/store/size failure | Raw fail-open |
| Session-level paired assignment | 10 Control / 10 Treatment |
| Objective evaluator | Exact sentinel versus source exit class |
| Source reduction | 93.52% |
| Uncached provider input | -35.77% aggregate; negative paired median |
| Task quality | 10/10 Control and Treatment |
| Retries / recalls | 0 / 0 |
| Observatory | Active Source and Downstream evidence separated |
| Default runtime policy | Still passthrough |

The next reducer may be researched only after this result is preserved. The expansion order remains
`cargo check`/`clippy`, then `rg`, then `git status`; none is implicitly admitted by this closure.
