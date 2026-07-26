# Profiles

Profiles restrict which enabled servers appear in the frontend tool catalog.
They do not rewrite `mcp.json`.

```toml
[profiles.default]
servers = ["*"]

[profiles.security]
servers = ["github", "ghidra", "burp"]
```

```text
hydian profiles list
hydian profiles show security
hydian profiles use security --dry-run
hydian profiles use security
```

The active profile persists in `config.toml`. `--profile <name>` overrides it
for one foreground invocation. The TUI Profiles screen previews servers and
visible tool count before activation.
