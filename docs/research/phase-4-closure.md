# Phase 4 closure: post-hoc context optimization

Phase 4 evaluated transformations after a ToolResult had already entered a
provider request.  This is a bounded conclusion about the current public and
shadow cohorts, not a claim that context optimization is universally futile.

| Surface | Result | Decision |
|---|---|---|
| Representation compression | The addressable share was below the materiality gate. | Do not activate. |
| Provider-native rewriting | Compatible search envelopes did not yield material reduction. | Do not activate. |
| Generic lossy projection | The structural upper bound did not justify an opaque representation. | Do not activate. |
| Historical ToolResult lifetime eviction | Best shadow net total-context reduction was 2.76%; recovery and cache risk add complexity. | Do not activate. |

The Phase 4 negative-results registry is the authoritative index of the
workload-scoped findings.  Future post-hoc work needs new evidence identifying
a materially different tool family or provider behavior; it must not restart
these candidates by default.

The next research surface is Phase 5 source-side optimization: reduce
command-specific noise before it becomes agent context, while measuring the
provider and task-level effects separately from output-byte reduction.
