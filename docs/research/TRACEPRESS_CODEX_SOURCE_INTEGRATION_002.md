# Tracepress Codex source-integration follow-up 002

## Scope

This record follows the Phase 5.0 hook-contract spike. It evaluates source-side
integration surfaces before any reducer, recovery store, or active forwarding
policy is enabled.

## Measured surfaces

| Surface | Observed result | Decision |
|---|---|---|
| `PreToolUse.updatedInput` wrapper | Operational, but an identity rewrite changed provider input before the proxy executed | Not causal for reducer measurement |
| PATH `cargo` shim | Agent exited before provider, hook, or source execution | Closed for this Codex runtime |
| `codex exec-server` | CLI exposes an experimental remote-executor service, not a local transparent interceptor | Do not exercise without an explicit executor protocol and authority review |

The `updatedInput` attribution smoke establishes that the first effect is in
Codex's treatment of the hook response, rather than Tracepress's command
execution. The PATH result establishes that Tracepress must not attempt to
make the shim work by weakening the sandbox or approval model.

## App-server protocol discovery

Codex 0.154's locally generated experimental app-server schema includes native
`command/exec` requests with a sandbox policy and streamed stdout/stderr output
notifications. It also models command-approval requests separately. This is a
promising observation surface because the native command, sandbox, and approval
lifecycles remain explicit.

The schema alone does not establish that an app-server client may replace a
terminal command output while preserving those lifecycles. No app-server client
or executor has been connected yet.

## Next safe spike

Protocol discovery only:

1. implement a stdio app-server protocol observer that records only event
   shapes and lifecycle metadata;
2. identify whether a command result can be observed or transformed while
   Codex retains native approval and sandbox ownership;
3. write a threat/contract note before connecting an executor;
4. require a byte-faithful A/A control before any source reducer.

No remote environment is registered, no executor is launched, and no output
content is persisted by this record.
