# Architecture

## Purpose

Hydian combines tools from ordinary MCP server configurations behind one
stable MCP endpoint. It is a local control point, not a hosted broker or policy
platform.

## Protocol baseline

V0.1 targets the stable MCP protocol revision `2025-11-25` and the official
Rust SDK's stable `rmcp` 2.x line. Streamable HTTP replaces the legacy
HTTP-plus-SSE transport. The frontend validates `Origin` when it is present and
binds to loopback by default.

Only the tools capability is advertised. Hydian does not pretend to support
resources, prompts, roots, sampling, elicitation, completion, or tasks.

## Component flow

```text
MCP clients
    |
    +-- Streamable HTTP /mcp
    +-- stdio
            |
      frontend adapters
            |
      application commands
            |
       tool catalog
       /          \
profiles        routing
                  |
        backend supervisor
        /                \
local stdio       remote Streamable HTTP
```

The same configuration loader, catalog, router, diagnostics, service manager,
exposure providers, and application commands serve the CLI and TUI. Widgets do
not start processes or write configuration directly.

## Sessions and concurrency

Hydian keeps one initialized client session per configured backend and shares
it across frontend clients. Stdio calls are serialized by default. A backend's
semaphore provides configurable concurrency without changing session
ownership.

This tools-only boundary intentionally avoids virtualizing client roots,
sampling callbacks, elicitation, or other client-specific state.

## Naming and routing

Tools are exposed as `<server>__<tool>`. Each component is sanitized to the MCP
tool-name guidance while retaining ASCII letters, digits, underscore, hyphen,
dot, and slash. Hydian rejects collisions after sanitization. The catalog preserves
the backend's original name, description, input schema, optional output schema,
annotations, and availability.

## Persistence

V0.1 persists configuration, logs, backups, service definitions, and an
atomically replaced runtime status file. It has no database or persistence
abstraction.

## Exposure and service boundaries

Tunnel providers are transparent adapters around operator-installed binaries.
Every adapter detects, validates, plans, starts, reports, and stops through a
common application command surface. Hydian never downloads provider agents.

Service behavior is platform-specific: systemd user units on Linux, a user
LaunchAgent on macOS, and a logon Scheduled Task on Windows. System-level
installation is always explicit.

## Primary references

- [MCP transports, revision 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- [Official Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [Ratatui](https://ratatui.rs/)
- [Tailscale Serve](https://tailscale.com/docs/reference/tailscale-cli/serve)
- [Tailscale Funnel](https://tailscale.com/docs/reference/tailscale-cli/funnel)
- [ngrok Agent API](https://ngrok.com/docs/agent/api)
- [Cloudflare Quick Tunnels](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/trycloudflare/)
