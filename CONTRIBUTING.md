# Contributing to Hydian

Hydian welcomes focused bug fixes, tests, documentation, and changes that keep
the v0.1 product small and dependable.

## Before opening a change

1. Discuss large protocol, configuration, or dependency changes in an issue.
2. Keep one deployable Rust binary and do not add a database, web UI, Docker
   requirement, or non-Rust runtime.
3. Preserve safe defaults without blocking explicit, acknowledged operator
   choices.
4. Never place credentials, tokens, private tool arguments, or unredacted
   support data in issues or test fixtures.

## Development checks

Use the stable Rust toolchain and run:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

Tests must not require real Tailscale, ngrok, Cloudflare, or OpenAI accounts.
Provider and service operations should be exercised through command plans and
fake process executors.

## Commit and review expectations

- Keep commits cohesive and leave the tree buildable.
- Explain user-visible configuration or protocol changes.
- Add focused tests for behavior changes.
- Avoid drive-by formatting and unrelated refactors.

By contributing, you agree that your contribution may be licensed under the
MIT OR Apache-2.0 terms used by this project.
