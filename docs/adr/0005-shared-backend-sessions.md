# ADR 0005: Shared backend sessions

- Status: Accepted
- Date: 2026-07-25

## Decision

V0.1 creates one initialized session per configured backend and shares it
across frontend clients. Per-backend semaphores bound concurrent tool calls;
stdio defaults to one concurrent call.

## Consequences

Backend process count and session behavior are predictable. A frontend client
cannot have distinct roots, sampling callbacks, elicitation state, or other
client-specific backend state. This limitation is documented rather than
hidden.
