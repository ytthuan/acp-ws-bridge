# acp-ws-bridge

[![CI](https://github.com/ytthuan/acp-ws-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/ytthuan/acp-ws-bridge/actions/workflows/ci.yml)
[![Coverage](https://github.com/ytthuan/acp-ws-bridge/actions/workflows/coverage.yml/badge.svg)](https://github.com/ytthuan/acp-ws-bridge/actions/workflows/coverage.yml)
[![Crates.io](https://img.shields.io/crates/v/acp-ws-bridge.svg)](https://crates.io/crates/acp-ws-bridge)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

WebSocket-to-stdio bridge for relaying [Agent Client Protocol (ACP)](https://agentclientprotocol.com) JSON-RPC messages between a remote client (e.g., iOS app) and GitHub Copilot CLI.

```
┌─────────────────┐        stdio         ┌─────────────────────┐       WebSocket        ┌─────────────────┐
│  GitHub Copilot │◄──────────────────►  │   acp-ws-bridge     │◄──────────────────────►│   Remote Client │
│  CLI (Host)     │   JSON-RPC/NDJSON    │   (Rust Server)     │   JSON-RPC/NDJSON      │   (iOS App)     │
└─────────────────┘                      └─────────────────────┘                        └─────────────────┘
```

## Features

- **Transparent relay** — passes all ACP JSON-RPC messages without modification
- **WebSocket + optional TLS** — secure remote connections
- **REST API** — session history, stats, Copilot CLI usage metrics and capabilities
- **Copilot CLI version detection** — detects CLI version at startup, exposes via API
- **Ping/pong keepalive** — detects dead connections
- **Session management** — tracks active sessions with `--resume` support

## Compatibility

| Copilot CLI Version | Status |
|---|---|
| **1.0.x** (GA) | ✅ Fully supported |
| 0.0.418 – 0.0.423 | ✅ Fully supported |
| < 0.0.418 (pre-GA) | ⚠️ Basic relay works; some features may be missing |

New ACP protocol methods (exitPlanMode.request, MCP elicitations, reasoning effort config) are transparently relayed without bridge changes.

## Quick Start

```bash
# Build
cargo build --release

# Run (starts Copilot CLI via stdio, listens on WebSocket port 8765)
cargo run -- --ws-port 8765

# With TLS (enables wss:// for WebSocket and HTTPS for REST API)
cargo run -- --ws-port 8765 --tls-cert cert.pem --tls-key key.pem

# Generate cert.pem/key.pem with mkcert (after `mkcert -install`)
mkcert -key-file key.pem \
  -cert-file cert.pem \
  localhost 127.0.0.1 ::1

# With custom API port
cargo run -- --ws-port 8765 --api-port 8766

# With an exact ACP command override (alias: --command)
cargo run -- --acp-command "copilot --acp --stdio --allow-all"

# With a custom Copilot data directory
cargo run -- --ws-port 8765 --copilot-dir /srv/copilot-data
```

## Install

```bash
# Install from crates.io
cargo install acp-ws-bridge
```

Prebuilt release binaries are published on GitHub Releases for:

- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `--ws-port` | 8765 | WebSocket listen port |
| `--api-port` | 8766 | REST API listen port |
| `--tls-cert` | — | TLS certificate path (enables wss:// and HTTPS) |
| `--tls-key` | — | TLS private key path |
| `--generate-cert` | — | Generate self-signed certificate and exit |
| `--cert-hostnames` | `localhost,127.0.0.1` | Hostnames for self-signed certificate |
| `--copilot-path` | `copilot` | Path to Copilot CLI executable when using the default spawned command |
| `--copilot-args` | — | Extra args appended to the default spawned Copilot CLI command |
| `--acp-command`, `--command` | — | Exact Copilot ACP command override, parsed without shell execution |
| `--copilot-mode` | `stdio` | Copilot transport mode (`stdio` or `tcp`) |
| `--copilot-host` | `127.0.0.1` | Copilot CLI host in TCP mode |
| `--copilot-port` | `3000` | Copilot CLI port in TCP mode |
| `--spawn-copilot` | `true` | Disable this to connect to an already-running Copilot CLI instance |
| `--copilot-dir` | `~/.copilot` | Copilot data directory for session history, session-state, and stats cache files |

When `--tls-cert` and `--tls-key` are provided, both the WebSocket server (wss://) and the REST API (HTTPS) use the same TLS configuration.

If `--acp-command` / `--command` is set, it takes precedence over `--copilot-path` and `--copilot-args`. The provided command is treated as an exact override, so it must already include the ACP transport flags you want the spawned process to use, and in TCP mode it must match the configured `--copilot-port`.

## REST API

The REST API runs on a separate port (default: `--ws-port` + 1).

| Endpoint | Description |
|---|---|
| `GET /health` | Health check with bridge version, Copilot CLI version, uptime |
| `GET /api/sessions` | List active WebSocket sessions |
| `GET /api/sessions/:id` | Get session details |
| `DELETE /api/sessions/:id` | Delete a session |
| `GET /api/sessions/:id/commands` | Get cached ACP commands for a session |
| `GET /api/stats` | Aggregate session statistics |
| `GET /api/copilot/info` | Copilot CLI version, path, mode, GA status, feature capabilities |
| `GET /api/copilot/usage` | Copilot CLI usage statistics (model usage, tool executions) |
| `GET /api/history/sessions` | Historical sessions from the configured Copilot data directory |
| `GET /api/history/sessions/:id/turns` | Session conversation turns |
| `GET /api/history/stats` | Aggregate historical statistics |

## Project Docs

- [Contributing guide](CONTRIBUTING.md)
- [Code of conduct](CODE_OF_CONDUCT.md)
- [Security policy](SECURITY.md)
- [Architecture](docs/architecture.md)
- [Deployment](docs/deployment.md)
- [Testing](docs/testing.md)
- [Release runbook](docs/release.md)

## Generate TLS Cert/Key

If you installed `acp-ws-bridge` from crates.io or GitHub Releases and just need `cert.pem` / `key.pem` for local TLS, `mkcert` is the simplest option on macOS and Linux.

### macOS

```bash
brew install mkcert
brew install nss
mkcert -install
```

`nss` is only needed if you want Firefox and other NSS-based clients to trust the local CA.

### Linux

Install `mkcert` from your distro package manager or the upstream release, plus the NSS tools package if you want Firefox and other NSS-based clients to trust the local CA.

```bash
# Debian / Ubuntu
sudo apt install mkcert libnss3-tools

# Fedora
sudo dnf install mkcert nss-tools

# Arch Linux
sudo pacman -S mkcert nss

# One-time local CA install
mkcert -install
```

Generate a certificate and key for every hostname or IP your clients will use:

```bash
mkcert -key-file key.pem \
  -cert-file cert.pem \
  localhost 127.0.0.1 ::1 192.168.0.100 bridge-host.example.com
```

Replace `192.168.0.100` and `bridge-host.example.com` with the LAN IP and DNS name clients actually use.

If a client connects from another device, that device must also trust the `mkcert` root CA. Use `mkcert -CAROOT` to locate the CA files if you need to import them on another machine or device.

### Windows (PowerShell + OpenSSL)

```powershell
openssl req -x509 -newkey rsa:2048 -sha256 -days 365 -nodes `
  -keyout key.pem `
  -out cert.pem `
  -subj "/CN=localhost" `
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

Once the files exist, start the installed binary with:

```bash
acp-ws-bridge --ws-port 8765 --tls-cert cert.pem --tls-key key.pem
```
The bridge loads these PEM files directly and should not need macOS keychain identity access for the server certificate.

Run in background with:

```bash
acp-ws-bridge  --ws-port 8700 --acp-command "copilot --acp --stdio" --tls-cert cert.pem --tls-key key.pem  --log-level debug >~/logs/acp-ws-8700.log 2>&1 &
```

## Release Process

Releases are tag-driven and deterministic:

1. Bump `version` in `Cargo.toml` (manual semver).
2. Merge to `main`.
3. Create and push a matching tag: `vX.Y.Z`.

The release workflow verifies the tag matches `Cargo.toml`, runs strict checks (`fmt`, `clippy -D warnings`, `test`, package dry-run), publishes to crates.io using the `CARGO_REGISTRY_TOKEN` GitHub secret, and uploads platform binaries to GitHub Releases.

## Contributing and Support

- Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.
- Use the GitHub bug report and feature request templates for public issues.
- Report vulnerabilities privately according to [SECURITY.md](SECURITY.md).
- Community interactions in this repository follow [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Roadmap

Near-term repository improvements are focused on OSS maintainability:

- keep contributor-facing docs current as the bridge evolves
- improve quality visibility through the dedicated coverage workflow
- evaluate future release automation and additional examples only after the contributor baseline is stable

## License

MIT
