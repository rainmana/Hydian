# Draft: One endpoint for every MCP server — introducing Hydian

> **Publication status:** Draft for a personal blog and Medium. Verify release
> links, installation commands, screenshots, and version claims before posting.

If you use several AI coding clients, you may also maintain the same collection
of Model Context Protocol (MCP) servers several times. Each client needs its own
configuration, each server has its own lifecycle, and a broken integration can
make the whole setup feel fragile.

I built **Hydian** to make that boundary boring: configure MCP servers once and
point compatible clients at one local endpoint.

## Why the name?

In *Star Wars*, the Hydian Way is a major hyperspace route connecting distant
systems. The name fit the idea of one well-known route to many independent MCP
servers. Hydian is an unofficial open-source project and is not affiliated with
or endorsed by Lucasfilm or Disney.

## What it does

Hydian is a native Rust, local-first MCP multiplexer. It starts or connects to
stdio and Streamable HTTP backends, qualifies tool names so collisions remain
predictable, and exposes the combined catalog over loopback Streamable HTTP or
stdio. A backend failure is isolated rather than taking every other server
offline.

The v0.1 boundary is intentionally narrow. Hydian multiplexes tools, shares
backend sessions, has no database or web UI, and does not bundle a tunnel or
terminate HTTPS. External access remains an explicit operator choice.

## A five-minute workflow

```console
hydian init
hydian import /path/to/existing-mcp.json
hydian import /path/to/existing-mcp.json --apply
hydian doctor
hydian serve
```

The first import is a preview. Mutating operations emphasize plans, backups,
and explicit application because infrastructure tools should make their effects
clear before changing a machine.

Clients can then connect to `http://127.0.0.1:7337/mcp`, or launch
`hydian stdio` when command transport is a better fit.

## Safety without taking away control

Hydian's operating philosophy is: nothing dangerous happens by accident, and
nothing intentional is prohibited merely because it is dangerous. Loopback is
the default. Origin validation is enabled. Diagnostics explain risks and
remediation. Exposure is delegated to transparent provider commands instead of
hiding a tunnel inside the executable.

That is different from declaring every advanced workflow unsafe. An informed
operator can choose a different bind address or exposure provider, but Hydian
should show exactly what that choice means.

## Building in the open

The repository includes layered Rust tests, cross-platform CI, reproducible
release archives, structured issue and pull-request workflows, and a docs site
built from Markdown. Architectural decisions are recorded alongside the code so
future changes can explain not only *what* changed, but *why*.

## What comes next

The next milestone is feedback: which client imports are most valuable, which
diagnostics save the most time, and where real MCP servers differ from the
fixtures. The roadmap deliberately keeps resources, prompts, and larger policy
features out of v0.1 until the tools-only foundation proves dependable.

If that problem sounds familiar, try Hydian, read the security model, and open
an issue with a redacted reproduction. Contributions that keep the tool small,
clear, and dependable are welcome.

---

**Before publishing:** add the repository and documentation URLs, release
install instructions, one terminal screenshot, tested client versions, and a
short author bio. For Medium, use the subtitle “A local-first MCP multiplexer
for a growing collection of AI tools” and tags such as *MCP*, *Rust*,
*Developer Tools*, *AI*, and *Open Source*.
