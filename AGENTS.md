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
├── main.rs          → Entry point, config parsing, server startup
├── config.rs        → CLI config struct (clap)
├── bridge.rs        → WebSocket/HTTP server wiring
├── ws.rs            → WebSocket session relay (stdio/tcp, keepalive)
├── session.rs       → Session lifecycle, idle timeout, command cache
├── copilot.rs       → Copilot CLI process spawn (stdio/tcp modes)
└── api.rs           → REST API for health/sessions/stats/history
Cargo.toml
```

---

## Build & Test

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
bash scripts/validate_agents.sh
cargo run -- --ws-port 8765          # stdio mode (default)
cargo run -- --ws-port 8765 --log-level debug
```

---

## Agent

For complex tasks, dispatch to `.github/agents/rust-bridge-agent.agent.md`.
For code review, dispatch to `.github/agents/code-reviewer.agent.md`.
