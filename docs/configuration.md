# Configuration

Hydian reads `config.toml` for gateway behavior and `mcp.json` for MCP server
definitions. Writes use atomic replacement and preserve the previous file
under `backups/`.

```toml
version = 1

[listener]
host = "127.0.0.1"
port = 7337
path = "/mcp"

[runtime]
active_profile = "default"
startup_timeout_seconds = 20
request_timeout_seconds = 120
shutdown_grace_seconds = 10

[naming]
separator = "__"

[restart]
enabled = true
initial_delay_ms = 500
maximum_delay_seconds = 30
maximum_restarts_per_minute = 5

[logging]
level = "info"
format = "pretty"
retain_days = 14

[servers.defaults]
max_concurrent_calls = 1

[profiles.default]
servers = ["*"]

[security]
validate_origin = true
allowed_origins = []

[acknowledgements]
non_loopback_without_auth = false
disabled_origin_validation = false
```

An empty `allowed_origins` derives the three loopback origins for the
configured port. Requests without `Origin` remain valid for native MCP
clients. Browser requests with an unlisted origin are rejected.

## MCP definitions

```json
{
  "mcpServers": {
    "local": {
      "type": "stdio",
      "command": "my-mcp-server",
      "args": ["--mode", "mcp"],
      "env": {"TOKEN": "env:LOCAL_TOKEN"},
      "cwd": "C:\\work",
      "startupTimeoutSeconds": 20,
      "requestTimeoutSeconds": 120
    },
    "remote": {
      "type": "streamable-http",
      "url": "https://example.invalid/mcp",
      "headers": {
        "Authorization": "env:REMOTE_TOKEN",
        "X-Certificate": "file:C:\\secrets\\certificate.txt"
      }
    }
  }
}
```

Hydian also accepts a top-level `servers` object. Unknown definition fields
are preserved by imports and reported diagnostically. Literal header values
are allowed but `hydian doctor` warns about likely plaintext credentials.

`hydian init` writes `config.schema.json` beside the configuration.
