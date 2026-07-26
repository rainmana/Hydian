# ADR 0001: No database in v0.1

- Status: Accepted
- Date: 2026-07-25

## Decision

Hydian v0.1 will not use SQLite, libSQL, Turso, an ORM, migrations, or a
persistence abstraction. Persistent state is limited to `mcp.json`,
`config.toml`, logs, backups, service definitions, and ephemeral
`run/status.json`.

## Consequences

Configuration remains inspectable and recoverable with ordinary tools. Atomic
replacement and backups are sufficient for the v0.1 mutation volume.

A database may become justified for historical health, cached manifests,
usage history, audit events, annotations, remote synchronization, or dynamic
configuration transactions. Local libSQL/Turso is one future candidate, not a
current dependency.
