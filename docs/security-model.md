# Security model

Hydian binds to loopback HTTP by default. It has no v0.1 TLS terminator or
frontend authentication. A non-loopback listener can expose every tool in the
active profile as plaintext to reachable clients, so startup requires:

```toml
[acknowledgements]
non_loopback_without_auth = true
```

Origin validation follows the Streamable HTTP DNS-rebinding requirement.
Disabling it requires a separate acknowledgement. These acknowledgements are
factual records of operator intent, not claims that the configuration became
safe.

Backend header values support `env:` and `file:` references. Literal values
remain compatible with imported configurations but are reported by doctor.
Hydian redacts likely secret keys and resolved values from status, logs,
diagnostics, and provider state. Tool arguments are not logged at info level.

Backend tool calls are not retried after ambiguous timeout or transport
failure because a non-idempotent call may already have executed.

Run:

```text
hydian doctor --strict
hydian explain non-loopback-without-auth
hydian explain origin-validation
hydian explain plaintext-secrets
hydian explain provider-exposure
```
