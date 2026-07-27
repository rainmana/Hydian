# Hydian

One endpoint. Every MCP server.

Hydian is a small, local-first MCP multiplexer. It starts or connects to the
MCP servers you already use and exposes their tools through one stable endpoint
for ChatGPT, Codex, Claude, VS Code, Cursor, and other MCP clients.

Configure MCP servers once. Point every client at Hydian.

Hydian is not an MCP marketplace, an identity provider, a hosted service, an
enterprise policy platform, or a replacement for MCP servers. It does not
require Docker or a language runtime other than the Hydian executable itself.

## Why “Hydian”?

The name is inspired by the **Hydian Way**, the major hyperspace route in
*Star Wars* that connects otherwise distant systems. In the same spirit,
Hydian provides one dependable route to a collection of independent MCP
servers. The project is unofficial and is not affiliated with or endorsed by
Lucasfilm or Disney.

> **Project status:** v0.1 is implemented but not yet published. Treat the
> configuration surface as pre-release until a tagged release exists.

## Operating philosophy

Nothing dangerous happens by accident.
Nothing intentional is prohibited merely because it is dangerous.

Hydian provides safe defaults, precise warnings, and better options. It does
not take control of the operator's computer away from them.

## Five-minute walkthrough

```powershell
hydian init
hydian import C:\path\to\existing-mcp.json
hydian import C:\path\to\existing-mcp.json --apply
hydian doctor
hydian serve
```

The local Streamable HTTP endpoint is:

```text
http://127.0.0.1:7337/mcp
```

Running `hydian` in an interactive terminal opens the dashboard. Headless
operation uses `hydian serve`; command-oriented MCP clients can launch
`hydian stdio`.

## v0.1 boundary

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

The complete documentation is also maintained as an mdBook-compatible site in
[`docs/`](docs/SUMMARY.md). Every Markdown change is previewable locally with
`mdbook serve --open`, and `main` is published automatically to GitHub Pages.

## Client configuration

Current Codex TOML:

```toml
[mcp_servers.hydian]
url = "http://127.0.0.1:7337/mcp"
startup_timeout_sec = 20
tool_timeout_sec = 120
```

Claude/Cursor-style command configuration:

```json
{
  "mcpServers": {
    "hydian": {
      "command": "hydian",
      "args": ["stdio"]
    }
  }
}
```

VS Code-style command configuration:

```json
{
  "servers": {
    "hydian": {
      "type": "stdio",
      "command": "hydian",
      "args": ["stdio"]
    }
  }
}
```

Client formats change. Confirm the format against the client version you use.
ChatGPT cannot reach a loopback endpoint on your computer directly; see
[docs/exposure.md](docs/exposure.md) for provider adapters and the OpenAI
tunnel-client handoff.

## Commands

```text
hydian init
hydian import <path> [--format auto|claude|vscode|cursor|codex] [--apply]
hydian serve [--tui]
hydian stdio
hydian tui
hydian doctor [--strict]
hydian status
hydian endpoint [--format url|json|openai]
hydian servers list|show|start|stop|restart
hydian tools list|search|show
hydian profiles list|show|use
hydian service install|uninstall|start|stop|restart|status
hydian expose plan|start|stop|status
hydian explain <topic>
hydian completion <powershell|bash|zsh|fish>
```

Every configuration, service, and exposure mutation has a preview/dry-run
path. `hydian import` requires `--apply`.

## Configuration locations

| Platform | Hydian home |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Hydian\` |
| Linux | `~/.hydian/` |
| macOS | `~/.hydian/` |

Override the layout with `HYDIAN_HOME` or `--home`. Override the primary files
with `--config` and `--mcp-config`.

## Limitations

- v0.1 multiplexes tools only.
- One backend session is shared by all frontend clients.
- Hydian does not terminate HTTPS or authenticate frontend clients.
- Exposure behavior and identity guarantees belong to the selected provider.
- Cloudflare Quick Tunnels are experimental because they do not support SSE.
- Unavailable backends are isolated, but their tools are unavailable.
- Large tool catalogs consume client context; use profiles to keep catalogs
  focused.
- There is no database, web UI, legacy SSE-only backend, or embedded tunnel
  agent.

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT License

at your option.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow, test
layers, issue triage, pull-request checklist, and release process. Bug reports
and feature proposals use the repository's structured GitHub issue forms.
