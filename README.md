# Hydian

One endpoint. Every MCP server.

Hydian is a small, local-first MCP multiplexer. It starts or connects to the
MCP servers you already use and exposes their tools through one stable endpoint
for ChatGPT, Codex, Claude, VS Code, Cursor, and other MCP clients.

Configure MCP servers once. Point every client at Hydian.

Hydian is not an MCP marketplace, an identity provider, a hosted service, an
enterprise policy platform, or a replacement for MCP servers. It does not
require Docker or a language runtime other than the Hydian executable itself.

> **Project status:** Hydian is under active development. The v0.1 command and
> protocol surfaces are not yet released.

## Operating philosophy

Nothing dangerous happens by accident.
Nothing intentional is prohibited merely because it is dangerous.

Hydian provides safe defaults, precise warnings, and better options. It does
not take control of the operator's computer away from them.

## Planned v0.1 boundary

- One native Rust executable.
- Local stdio and remote Streamable HTTP backends.
- One loopback Streamable HTTP frontend at
  `http://127.0.0.1:7337/mcp`, plus a stdio frontend.
- Tool capabilities only: `tools/list`, `tools/call`, and
  `notifications/tools/list_changed` when available.
- Deterministic names such as `github__search_issues`.
- Shared backend sessions and serialized stdio calls by default.
- No database, built-in HTTPS termination, web UI, or bundled tunnel agent.

The detailed design is in [docs/architecture.md](docs/architecture.md).

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT License

at your option.
