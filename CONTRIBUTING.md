# Contributing to Tracepress

Thank you for improving Tracepress. Changes should preserve its local-first, bounded, metadata-only,
and fail-open contracts.

## Before changing code

- Read the relevant contract under `docs/design/`; do not infer a runtime policy from a research
  report alone.
- Keep forwarding, observation, shadow evaluation, and active mutation as separate concerns.
- Never add prompt, response, tool-result, command output, credentials, raw URLs, or provider bodies
  to logs, debug output, durable records, fixtures, or reports.
- Avoid broad dependency additions. Explain why a new crate is needed and disable unused default
  features where practical.

Use a focused branch or worktree and preserve unrelated local changes. Make commits small enough
that their behavior and evidence boundary can be reviewed independently.

## Tests and checks

Add the narrowest meaningful test at the contract boundary you changed. Include normal behavior,
typed error paths, resource limits, and privacy/fail-open properties where applicable. Tests must be
deterministic and must not require network access or real user state.

Run the required checks in [docs/development.md](docs/development.md). At minimum, a change should
pass formatting, Clippy with warnings denied, the affected package tests, and the MSRV check. Run
the whole nextest suite and doctests before requesting review unless the change is documentation
only; disclose any check you could not run.

Coverage is a regression signal, not a substitute for assertions. New tests should verify observable
behavior rather than execute lines only. Resource-sensitive latency tests already have isolated
nextest profiles and must not be made less strict to hide contention.

## Pull requests

A pull request should describe:

- the contract or problem being changed;
- privacy, forwarding, compatibility, and resource-bound implications;
- the checks run and their exact scope;
- any intentionally deferred work or evidence the change does not establish.

Do not combine generated experiment artifacts, runtime policy changes, and unrelated refactors in
one review unless they are inseparable.

## Security reports

Do not open a public issue containing vulnerability details. Follow [SECURITY.md](SECURITY.md).
