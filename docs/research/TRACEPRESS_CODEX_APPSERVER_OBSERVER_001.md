# Tracepress Codex app-server observer 001

## Result

Codex 0.154.0 accepted a stdio app-server `initialize`, an ephemeral
`thread/start`, and one `turn/start` under `read-only` sandbox with approval
policy `never`.

The controlled turn requested one harmless `printf`. Metadata-only observation
recorded:

- one command-execution item lifecycle (`item/started` plus `item/completed`);
- 29 characters of aggregated command output in the completed item;
- zero `item/commandExecution/outputDelta` notifications;
- a completed turn with no approval or sandbox override by Tracepress.

The observer persisted neither command text nor output content. It retained only
event names, item types, and size counters.

## Decision

App-server is a valid native observation surface for command lifecycle and
agent-visible result size. The measured output was available in the completed
command item, after Codex had assembled it. No protocol evidence currently
shows a client-controlled response or substitution point that could replace
that output before it re-enters the agent turn.

Therefore this spike does not reopen source reduction. The next gate is schema
proof of a pre-context command-result response mechanism. If none exists,
app-server remains useful for causal observability only, not transformation.

The complete experimental server-to-client request union was checked. Its
command-execution request is approval-only
(`item/commandExecution/requestApproval`); no request/response method accepts a
replacement command result. Client `command/exec` launches a separate command
and is not the result channel for agent-originated command items. The gate is
therefore negative for Codex 0.154.0.
