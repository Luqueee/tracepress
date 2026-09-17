# TRACEPRESS_EXPLICIT_AA_SMOKE_001

Status: **infrastructure passed; cohort required**. Both arms invoked the same
explicit Tracepress tool, recorded one source execution, emitted 20,674 raw
bytes, and used no hook or reducer. Both tasks succeeded without provider
errors or timeout.

Provider requests varied 2 versus 3, so this N=1 smoke is not an equivalence
decision. The alternating N=10 A/A is required.
