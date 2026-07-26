# ADR 0004: External exposure providers

- Status: Accepted
- Date: 2026-07-25

## Decision

Hydian integrates with operator-installed `tailscale`, `ngrok`, and
`cloudflared` executables and supports a custom command adapter. It does not
bundle, download, authenticate, or reimplement a provider agent.

Every provider exposes a transparent command plan before execution, including
scope, authentication facts, TLS termination facts, and known limitations.
Provider-native arguments remain available after `--`.

## Consequences

Provider accounts remain optional and outside Hydian's custody. Cloudflare
Quick Tunnels are development-only and experimental for MCP because current
Cloudflare documentation says they do not support SSE.
