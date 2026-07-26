# ADR 0002: Loopback HTTP first

- Status: Accepted
- Date: 2026-07-25

## Decision

Hydian's default frontend is Streamable HTTP at
`http://127.0.0.1:7337/mcp`. V0.1 does not terminate TLS. IPv6 loopback and
explicit arbitrary bind addresses are supported.

A non-loopback plaintext listener without authentication requires a persisted
acknowledgement after Hydian explains which interfaces, tools, and clients are
affected. Origin validation is enabled by default.

## Consequences

The default is appropriate for local clients and external tunnel agents.
Operators retain the ability to use another network design deliberately.
Hydian must never describe an acknowledged plaintext listener as protected.
