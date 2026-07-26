# Troubleshooting

## Backend executable not found

Run `hydian doctor`. Install the executable or change its `command` in
`mcp.json`. Hydian launches commands directly, without an intermediary shell,
so Windows command shims may need their actual `.cmd` or `.exe` name.

## Backend is degraded

Inspect `hydian status`, the TUI Servers screen, and `<HYDIAN_HOME>/logs/`.
Healthy backends remain callable. Restart attempts use bounded exponential
backoff and a per-minute ceiling.

## HTTP client cannot initialize

Confirm `http://127.0.0.1:7337/healthz`, then `/readyz`. Browser-originated
requests must use an allowed `Origin`. Native MCP clients normally omit it.

## TUI is not shown

The default dashboard requires interactive stdin and stdout, `TERM` other than
`dumb`, and no `--plain`. Use `hydian tui` to request it explicitly or
`hydian serve` for redirected/headless operation.

## Windows Rust build says `link.exe` is missing

Install Visual Studio Build Tools with the MSVC C++ toolset and Windows SDK.
Frontend source checks can pass without those native linker components, but a
Windows executable cannot be linked normally.

## Status file is stale

`run/status.json` is an atomic operational snapshot, not a database. Start
`hydian serve` and check the generated timestamp.
