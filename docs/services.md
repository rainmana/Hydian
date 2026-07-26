# Background services

Always inspect the exact definition first:

```text
hydian service install --dry-run
```

- Windows uses a least-privilege current-user Scheduled Task triggered at
  logon. It does not request or store a password.
- Linux uses a user-level systemd unit by default.
- macOS uses a per-user LaunchAgent by default.

`--system` explicitly requests system-level semantics. Linux and macOS then
require privileged file access. Windows system-service mode is deliberately
not claimed in v0.1; the command reports that limitation.

```text
hydian service install
hydian service start
hydian service status
hydian service restart --dry-run
hydian service stop
hydian service uninstall --dry-run
```

Installation resolves the current executable and Hydian home to absolute
paths, refuses temporary executables unless acknowledged, writes the native
definition, and verifies it with the platform status command. OS semantics are
not presented as identical.
