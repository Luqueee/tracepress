# Tracepress Instruction Source Attribution 016

## Decision

**PASS structural source attribution calibration. Keep every active instruction policy blocked.**

Four controlled public arms completed three sessions each under `codex-cli 0.155.0` and
`gpt-5.6-luna`. Every arm produced six provider requests with six complete snapshots. The
repository, task set, sandbox, approval behavior, model, and runtime were fixed.

## Structural attribution

| Stable class | Estimated tokens per provider request |
|---|---:|
| Common to all four arms | 5,761 |
| Present only when user config is loaded | 15,749 |
| Present only when user config is ignored | 1,540 |
| Added by controlled workspace `AGENTS.md` marker | 0 |
| Removed/replaced by workspace marker | 0 |
| Added by strict-config developer marker | 57 |
| Removed/replaced by developer marker | 0 |

The explicit `developer_instructions` marker validates the attribution mechanism: its known source
created a positive 57-token stable class without exposing its fingerprint or content. The workspace
`AGENTS.md` marker created no structural delta when combined with `--ignore-user-config`; that arm
is a scoped negative result and cannot calibrate workspace instruction attribution in this mode.

The 15,749-token class is attributed only to the **effect of loading user configuration versus
`--ignore-user-config`**. It is not claimed to be removable config-file text. The mode change may
alter plugins, apps, memories, generated instructions, defaults, or composition boundaries.

## Arm evidence

| Arm | Requests | Stable tokens/request | Provider input | Cached | Uncached |
|---|---:|---:|---:|---:|---:|
| User config | 6 | 21,510 | 144,260 | 139,776 | 4,484 |
| Ignore user config | 6 | 7,301 | 81,939 | 66,048 | 15,891 |
| Ignore config + workspace marker | 6 | 7,301 | 82,126 | 51,968 | 30,158 |
| Ignore config + developer marker | 6 | 7,358 | 82,094 | 65,024 | 17,070 |

Provider usage is adjacent observational evidence. Arms ran sequentially and have different cache
states, so these totals are not causal savings. In particular, fewer locally estimated stable
tokens did not imply lower observed uncached input in this cohort.

## Integrity and privacy

- Every arm proved its exact config/profile contract in its execution artifact.
- Provider-request, latest-snapshot, and complete-snapshot counts matched in every arm.
- Only developer text leaves with complete estimates and fingerprints participated.
- Stable classes had identical multiplicity and token weight in every request of their arm.
- Fingerprints were intersected ephemerally and never entered the report.
- No instruction text, prompt, command, path, response, semantic path, config contents,
  authentication material, session ID, or request ID was persisted.
- No global config or authentication file was modified or copied.

## Consequence

Phase 6.3 identifies a large config-loading-associated surface and validates explicit developer
source attribution. It does not identify a safe removal candidate. A later phase must decompose the
user-config effect into independently controlled layers and establish a cache-aware A/A baseline
before considering any active instruction policy.
