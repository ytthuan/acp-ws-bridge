# AGENTS.md

> **You are a Rust systems engineer working on `acp-ws-bridge`.**
> Implement, build, test, and maintain this codebase directly. No orchestration needed.

---

## Project Overview

`acp-ws-bridge` is a Rust WebSocket-to-stdio bridge for relaying ACP JSON-RPC messages between the Remo iOS app and GitHub Copilot CLI.

```
┌──────────────────┐       stdio/NDJSON      ┌──────────────────┐     WebSocket/NDJSON    ┌─────────────────┐
│  GitHub Copilot  │◄───────────────────────►│  acp-ws-bridge   │◄───────────────────────►│  Remo iOS App   │
│  CLI (Host)      │                          │  (Rust Server)   │                          │  (SwiftUI)      │
└──────────────────┘                          └──────────────────┘                          └─────────────────┘
       ACP Host                                Transport Bridge                               ACP Client
```

**Core principle:** Transparent relay — passes ALL messages without modification. WebSocket ↔ stdio/TCP, byte-for-byte.

---

## Tech Stack

| Aspect | Library |
|---|---|
| Async runtime | `tokio` |
| WebSocket | `tokio-tungstenite` |
| CLI parsing | `clap` |
| Logging | `tracing` |
| Serialization | `serde` + `serde_json` |
| Errors | `thiserror` + `anyhow` |

---

## Key Files

```
src/
├── main.rs          → Entry point, CLI arg parsing
├── config.rs        → Config struct (clap)
├── server.rs        → WebSocket server, connection handling
├── relay.rs         → stdio/TCP ↔ WebSocket relay logic
├── copilot.rs       → Copilot CLI spawn (stdio/tcp modes)
└── error.rs         → BridgeError types
Cargo.toml
```

---

## Build & Test

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
cargo run -- --ws-port 8765          # stdio mode (default)
cargo run -- --ws-port 8765 --log-level debug
```

---

## Agent

For complex tasks, dispatch to `.github/agents/rust-bridge-agent.agent.md`.
For code review, dispatch to `.github/agents/code-reviewer.agent.md`.
