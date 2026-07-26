# ADR 0003: Tools-only multiplexing

- Status: Accepted
- Date: 2026-07-25

## Decision

V0.1 advertises and routes only `tools/list`, `tools/call`, and tool-list
change notifications when available. It does not advertise resources, prompts,
sampling, roots, elicitation, completion, or tasks.

## Consequences

The multiplexer can preserve backend tool schemas and results without
inventing semantics for client-specific state. Adding another MCP capability
requires a separate design that explains state ownership and session
virtualization.
