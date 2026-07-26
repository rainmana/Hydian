# Importing MCP configurations

Hydian imports Claude/Cursor JSON (`mcpServers`), VS Code JSON (`servers`), and
current Codex TOML (`mcp_servers`). The source is never changed.

```text
hydian import config.json
hydian import config.json --format vscode
hydian import ~/.codex/config.toml --format codex
hydian import config.json --on-conflict rename --apply
```

Without `--apply`, the result is a preview. Conflict choices are `skip`,
`replace`, and deterministic `rename`. Applying an import reports the target,
backup, and rollback action. Duplicate names in source JSON are errors instead
of last-value-wins behavior.
