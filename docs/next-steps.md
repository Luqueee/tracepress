# Project next steps

This document records the maintenance work intentionally left after the Rust project-practices
baseline. It is ordered by prerequisite rather than implementation difficulty.

## In scope next

### 1. Complete the distribution license

Add the canonical `LICENSE-MIT` and `LICENSE-APACHE` texts matching the workspace declaration
`MIT OR Apache-2.0`. This is required before publishing binaries, source archives, packages, or a
public release. Confirm the copyright attribution with the repository owner rather than inventing
one during automation work.

Completion evidence:

- both license files are present at the repository root;
- package metadata and the README point to the same license expression;
- a produced source archive contains both files.

### 2. Establish a private security-reporting channel

Enable GitHub Private Vulnerability Reporting and replace the temporary contact fallback in
`SECURITY.md` with the repository's private report link. Do not solicit vulnerability details in a
public issue.

Completion evidence:

- the GitHub repository reports private vulnerability reporting as enabled;
- the private reporting link works for a non-maintainer;
- `SECURITY.md` states the supported versions and the real private contact path.

### 3. Define the release contract before CD

Write down the version/tag convention, supported target matrix, dashboard-asset packaging decision,
archive layout, checksums, signing or provenance mechanism, and rollback/withdrawal procedure.
Only then add a release workflow. It must build from an immutable tag, rerun release gates, use
least-privilege permissions, and never publish from a pull request.

Completion evidence:

- a reviewed release contract exists in `docs/`;
- a dry run produces reproducible, inspectable artifacts without publishing them;
- the publishing job requires the intended protected environment or explicit maintainer approval;
- release checksums and provenance can be verified independently.

### 4. Continue backend coverage by risk

Keep the global 80% line floor and improve behavior coverage where externally observable branches
remain weakest. Prioritize the CLI orchestration boundary, IPC client/serialization errors, storage
publication and garbage-collection failures, and dashboard API query/error contracts. Raise the
global threshold only after the new baseline is stable.

Tests should continue to target typed failures, resource bounds, privacy, exact-byte forwarding,
and fail-open behavior. Do not add tests whose only assertion is that code executed.

## Explicitly deferred

### Dashboard browser E2E

Dashboard browser E2E and dashboard line-coverage work are outside the current scope. The existing
native SSR/component tests remain part of CI, but no Playwright or equivalent browser dependency
should be introduced until dashboard work is resumed explicitly.

When resumed, the browser harness must use synthetic fixtures, build the shipped Dioxus WASM
application, start `tracepress ui --fixture` on an ephemeral loopback port, wait for
`/api/v1/health`, and retain browser diagnostics on failure. It must never read the operational
database.

### Miri and Loom

Miri is low priority while workspace code forbids unsafe Rust. Loom would require concurrency seams
designed around modelled synchronization rather than wrapping the existing runtime tests. Adopt
either only for a named risk and a testable invariant, not as a nominal CI badge.
