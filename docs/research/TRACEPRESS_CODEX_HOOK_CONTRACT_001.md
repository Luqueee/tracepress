# TRACEPRESS_CODEX_HOOK_CONTRACT_001

## Scope

An isolated runtime contract spike for Codex CLI source-side command rewriting.
It used a temporary `CODEX_HOME`, a temporary workspace, and a temporary
`hooks.json`; it did not change the user's Codex configuration, project
instructions, or global hook configuration. The temporary hook recorded only
event name, tool name, command presence, command byte length, and test mode.

## Runtime

- Codex CLI: `0.154.0`
- Event: `PreToolUse`
- Matcher: `^Bash$`
- Hook config discovery: `$CODEX_HOME/hooks.json`
- Sandbox under test: `read-only`
- Executions: non-interactive `codex exec --ephemeral`

## Observed contract

| Probe | Observation | Result |
|---|---|---|
| Identity rewrite | The hook received `tool_name: Bash`; an identical `updatedInput.command` returned byte-identical stdout. | Pass |
| Visible mutation | A requested `printf ORIGINAL` was executed as `printf REWRITTEN`, and Codex received `REWRITTEN`. | Pass |
| Hook trust | An untrusted temporary hook was not run and the raw command executed. Explicit temporary trust bypass made the hook run. | Understood |
| Invalid JSON | Codex marked the hook complete and executed the original command. | Fail open |
| Empty hook output | Codex marked the hook complete and executed the original command. | Fail open |
| Non-zero hook exit | Codex marked the hook failed and executed the original command. | Fail open |
| Hook timeout | Codex marked the hook failed and executed the original command. | Fail open |
| Sandbox after rewrite | A rewritten `touch` under `read-only` was rejected with a read-only filesystem error; no file was created. | Pass |

`codex exec` reported `approval: never` both when invoked with `-a never` and
with `-a on-request`. This spike therefore does not establish interactive TUI
approval behavior. It does establish that a hook `permissionDecision: allow`
did not bypass the `read-only` sandbox after a rewrite.

## Decision

The required Phase 5 gate is satisfied for non-interactive Codex CLI: the
runtime invokes `PreToolUse`, applies `updatedInput.command`, recognizes the
`Bash` matcher, retains sandbox enforcement, and has understood raw-command
fallback behavior for hook failures.

This is not authorization to bypass native Codex policy. Phase 5 must keep
hook rewrites narrow, fail open on unsupported shell syntax, and rely on
Codex's normal sandbox and approval path for the rewritten command.
