# acp-ws-bridge

WebSocket-to-stdio bridge for relaying [Agent Client Protocol (ACP)](https://agentclientprotocol.com) JSON-RPC messages between a remote client (e.g., iOS app) and GitHub Copilot CLI.

```
┌─────────────────┐        stdio         ┌─────────────────────┐       WebSocket        ┌─────────────────┐
│  GitHub Copilot  │◄──────────────────►│   acp-ws-bridge     │◄──────────────────────►│   Remote Client  │
│  CLI (Host)      │   JSON-RPC/NDJSON   │   (Rust Server)     │   JSON-RPC/NDJSON      │   (iOS App)     │
└─────────────────┘                      └─────────────────────┘                        └─────────────────┘
```

## Features

- **Transparent relay** — passes all ACP JSON-RPC messages without modification
- **WebSocket + optional TLS** — secure remote connections
- **REST API** — session history, stats, Copilot CLI usage metrics
- **Ping/pong keepalive** — detects dead connections
- **Session management** — tracks active sessions with `--resume` support

## Quick Start

```bash
# Build
cargo build --release

# Run (starts Copilot CLI via stdio, listens on WebSocket port 8765)
cargo run -- --ws-port 8765

# With TLS
cargo run -- --ws-port 8765 --tls-cert cert.pem --tls-key key.pem

# With custom API port
cargo run -- --ws-port 8765 --api-port 8766
```

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `--ws-port` | 8765 | WebSocket listen port |
| `--api-port` | 8766 | REST API listen port |
| `--tls-cert` | — | TLS certificate path |
| `--tls-key` | — | TLS private key path |
| `--resume` | — | Resume a previous session |
| `--copilot-path` | `gh copilot` | Path to Copilot CLI |

## License

MIT
