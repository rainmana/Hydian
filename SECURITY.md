# Security Policy

## Supported versions

Hydian has not published a stable release. Security fixes currently target the
latest revision of the default branch.

## Reporting a vulnerability

Do not open a public issue for a vulnerability that could expose credentials,
execute unintended backend tools, bypass origin or bind-address safeguards, or
leave child processes running outside Hydian's supervision.

Until a private project security contact is published, use GitHub's private
security advisory flow for the repository. Include:

- the affected revision and platform,
- a minimal reproduction,
- the expected and observed behavior,
- the practical impact, and
- whether secrets or third-party systems were involved.

Remove real tokens, credentials, private tool inputs, and identifying logs.

## Scope notes

Hydian is a local multiplexer, not an authentication boundary. Operators are
responsible for the capabilities of configured MCP servers and any network
exposure they intentionally enable. Hydian's default listener is loopback-only
HTTP; v0.1 does not terminate TLS.
