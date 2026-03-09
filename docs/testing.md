# Testing

This repository uses a combination of local validation commands, inline Rust tests, and GitHub Actions checks.

## Primary validation commands

Run these commands before opening a pull request that changes runtime behavior, the REST API, or release/deployment logic:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash scripts/validate_agents.sh
cargo package --locked --allow-dirty --no-verify
```

## What is currently covered

The test suite covers the project's core behavior in these areas:

- `src/acp.rs` — ACP framing and JSON-RPC helpers
- `src/api.rs` — REST API behavior
- `src/config.rs` — CLI config parsing and defaults
- custom ACP command parsing and configuration precedence
- custom Copilot data directory path resolution
- `src/session.rs` — session tracking, counters, and idle handling
- `src/tls.rs` — rustls-based TLS config loading and self-signed certificate generation
- `src/ws.rs` — message extraction and relay support logic

In addition to Rust tests, CI also validates:

- formatting
- lint cleanliness with warnings treated as errors
- packaging dry-run behavior
- agent configuration integrity through `scripts/validate_agents.sh`

## When to add tests

Add or update tests when a change affects:

- ACP message parsing or framing
- WebSocket relay behavior
- session state, counters, or timeout handling
- REST API responses or routes
- CLI flags or default behavior
- custom command parsing or precedence
- Copilot data-directory resolution
- TLS/runtime setup logic

Docs-only changes usually do not require new Rust tests, but they should still keep existing checks green when practical.

## Release validation

Release tags re-run the important quality gates before publishing:

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo package --locked --no-verify`

The release workflow also builds platform artifacts for Linux, macOS, and Windows.

## Coverage visibility

The repository includes a dedicated GitHub Actions coverage workflow for additional quality visibility. It is intended to complement, not replace, the main CI checks.
