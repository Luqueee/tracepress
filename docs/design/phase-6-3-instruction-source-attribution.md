# Phase 6.3 instruction source attribution

Phase 6.3 attributes stable developer-text block classes by controlled source intervention. It does
not remove, rewrite, summarize, reorder, or select instructions in ordinary Tracepress sessions.

## Arms

The controlled public workload uses the same pinned repository, task set, model, sandbox, and
approval behavior in four isolated arms:

```text
user_config
    current Codex user configuration

ignore_user_config
    Codex --ignore-user-config; authentication remains in CODEX_HOME

ignore_user_config_workspace_marker
    same ignored user config plus a fixed public AGENTS.md instruction marker

ignore_user_config_developer_marker
    same ignored user config plus fixed public developer_instructions through strict config
```

`--ignore-user-config` means only what Codex 0.155.0 documents: `config.toml` is not loaded while
authentication still uses `CODEX_HOME`. It is not described as “minimal Codex” and does not prove
that built-in instructions, default features, plugins discovered elsewhere, or provider-managed
state are absent.

Both markers are calibration interventions. Their content is fixed and public and aligns with the
read-only task. The workspace marker exists only in the temporary checkout. The developer marker is
passed as an explicit strict-config override. A detectable developer-marker effect proves that the
attribution method can observe a known source; neither marker makes arbitrary instructions safe to
remove.

## Attribution unit

Only complete explicit `text + developer + human_authored` leaves participate. For each arm,
Tracepress computes an ephemeral map from exact fingerprint to the token estimate of one copy and
the number of distinct provider requests containing it.

A block is stable in an arm only when it appears in every observed provider request in that arm.
Cross-arm set operations then report aggregate token estimates for:

- stable blocks common to all arms;
- stable blocks present only with user configuration loaded;
- stable blocks added by the controlled workspace marker; and
- stable blocks added by the controlled developer marker; and
- stable blocks removed or replaced by each intervention.

Fingerprints are never serialized. Total, stable, added, and removed token estimates are aggregate
metadata only.

## Integrity gates

- Every provider request in each observed session must have one latest, complete, explicitly
  complete, correlated snapshot.
- Every eligible developer text leaf must have an exact fingerprint and token estimate.
- Every arm must complete the requested sessions successfully.
- Runtime, model, repository commit, task count, sandbox, and approval policy must match.
- The controlled developer-marker arm must produce a positive structural delta from the ignored
  user-config arm. Otherwise the attribution mechanism fails calibration and the phase stops. The
  workspace-marker arm is separately descriptive because project instruction discovery may be
  unavailable under the ignored user configuration.
- Provider usage remains adjacent evidence and is never converted into attributed savings.

Provider-request counts may vary with agent trajectory. Stable identities and one-copy token
estimates are therefore compared instead of raw cohort totals.

## Privacy boundary

The persisted report excludes instruction text, prompts, commands, paths, responses, fingerprints,
semantic paths, session IDs, request IDs, configuration contents, and authentication material. The
runner uses the existing authentication location without copying or printing credentials.

## Decision boundary

A positive calibration permits a source-attribution report. It does not permit active instruction
mutation. Any later candidate must identify an owner-controlled source layer, define an objective
quality evaluator, pass byte-faithful infrastructure A/A, and then pass an isolated session-level
control/treatment experiment.
