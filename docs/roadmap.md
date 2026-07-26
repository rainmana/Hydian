# Roadmap

The v0.1 boundary is intentionally small: tools-only multiplexing, shared
backend sessions, local HTTP/stdio frontends, and external system adapters.

Possible later work:

- client-specific backend session virtualization;
- resources and prompts after their client-state implications are designed;
- historical health, cached manifests, usage history, audit events, and
  annotations;
- dynamic configuration transactions and remote synchronization;
- native package-manager distribution;
- additional provider adapters.

A database becomes justified only with durable historical or synchronized
entities. Local libSQL/Turso is one future candidate, not a v0.1 dependency.
Native HTTPS, a web UI, an enterprise policy engine, and bundled provider
agents are not implied roadmap commitments.
