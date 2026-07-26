# Research baseline

Verified 2026-07-25 from official specifications, provider documentation,
project repositories, and crates.io metadata.

## Protocol and Rust dependencies

- Stable MCP revision: `2025-11-25`.
- Standard transports: stdio and Streamable HTTP. Streamable HTTP replaces the
  legacy HTTP-plus-SSE transport.
- HTTP servers must reject an invalid `Origin` header with HTTP 403 and should
  bind to loopback for local operation.
- MCP tool-name guidance permits ASCII letters, digits, underscore, hyphen,
  and dot, with a recommended length of 1 through 128 characters.
- Official Rust SDK stable release selected: `rmcp 2.2.0`. The crates.io
  `3.0.0-beta` line is intentionally not used for v0.1.
- Ratatui selected: `0.30.2`, using its default Crossterm backend and
  `TestBackend` for focused rendering tests.
- Cross-platform paths use `directories 6.0.0`. Hydian uses `BaseDirs` so it
  can honor the product's exact `~/.hydian` and `%LOCALAPPDATA%\Hydian`
  layouts rather than accepting `ProjectDirs` suffixes.
- `service-manager 0.11.0` supports systemd and launchd user levels, but its
  native Windows manager is the system Service Control Manager. Hydian's
  non-administrator Windows default therefore uses a Scheduled Task directly.

## Provider behavior

- Tailscale Serve: `tailscale serve --bg <target>`; status supports `--json`.
- Tailscale Funnel: `tailscale funnel --bg <target>`; status supports
  `--json`; Funnel terminates public TLS and is limited to supported ports.
- ngrok: `ngrok http <address-or-url>`; assigned endpoints are available from
  the local Agent API at `http://127.0.0.1:4040/api/endpoints`.
- Cloudflare Quick Tunnel:
  `cloudflared tunnel --url <local-url>`. Official documentation labels Quick
  Tunnels development-only and says they do not support Server-Sent Events.
  Hydian therefore labels quick mode experimental and does not claim full MCP
  compatibility.

## Client configuration

Codex currently defines MCP servers in TOML tables named
`[mcp_servers.<name>]`.

- Stdio fields include `command`, `args`, `env`, `env_vars`, `cwd`,
  `startup_timeout_sec`, and `tool_timeout_sec`.
- Streamable HTTP fields include `url`, `bearer_token_env_var`,
  `http_headers`, and `env_http_headers`.

No stable public OpenAI tunnel-client command reference was available in the
official Codex manual or OpenAI developer documentation search. V0.1
documentation therefore gives Hydian's local MCP endpoint without inventing a
tunnel-client invocation.

## Sources

- [MCP transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- [Official Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [Ratatui API](https://docs.rs/ratatui/latest/ratatui/)
- [service-manager API](https://docs.rs/service-manager/latest/service_manager/)
- [directories API](https://docs.rs/directories/latest/directories/)
- [Tailscale Serve CLI](https://tailscale.com/docs/reference/tailscale-cli/serve)
- [Tailscale Funnel CLI](https://tailscale.com/docs/reference/tailscale-cli/funnel)
- [ngrok Agent CLI](https://ngrok.com/docs/agent/cli)
- [ngrok Agent API](https://ngrok.com/docs/agent/api)
- [Cloudflare Quick Tunnels](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/trycloudflare/)
- [Windows Scheduled Tasks](https://learn.microsoft.com/windows/win32/taskschd/schtasks)
- [Apple launch agents](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html)
