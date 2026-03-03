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

# With TLS (enables wss:// for WebSocket and HTTPS for REST API)
cargo run -- --ws-port 8765 --tls-cert cert.pem --tls-key key.pem

# Generate a self-signed certificate
cargo run -- --generate-cert --cert-hostnames "localhost,127.0.0.1"

# With custom API port
cargo run -- --ws-port 8765 --api-port 8766
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
| `--copilot-path` | `copilot` | Path to Copilot CLI |

When `--tls-cert` and `--tls-key` are provided, both the WebSocket server (wss://) and the REST API (HTTPS) use the same TLS configuration.

## Generate TLS Cert/Key by Script

You can use either the built-in generator (`--generate-cert`) or platform scripts below.

### Cross-platform (recommended)

```bash
cargo run -- --generate-cert --tls-cert cert.pem --tls-key key.pem --cert-hostnames "localhost,127.0.0.1"
```

### macOS / Linux (OpenSSL)

```bash
#!/usr/bin/env bash
set -euo pipefail

CERT_FILE="${1:-cert.pem}"
KEY_FILE="${2:-key.pem}"
SUBJ="${3:-/CN=localhost}"

openssl req -x509 -newkey rsa:2048 -sha256 -days 365 -nodes \
  -keyout "${KEY_FILE}" \
  -out "${CERT_FILE}" \
  -subj "${SUBJ}" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"

chmod 600 "${KEY_FILE}"
echo "Generated ${CERT_FILE} and ${KEY_FILE}"
```

### Windows (PowerShell + OpenSSL)

```powershell
param(
  [string]$CertFile = "cert.pem",
  [string]$KeyFile = "key.pem",
  [string]$Subject = "/CN=localhost"
)

openssl req -x509 -newkey rsa:2048 -sha256 -days 365 -nodes `
  -keyout $KeyFile `
  -out $CertFile `
  -subj $Subject `
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"

Write-Host "Generated $CertFile and $KeyFile"
```

## Release Process

Releases are tag-driven and deterministic:

1. Bump `version` in `Cargo.toml` (manual semver).
2. Merge to `main`.
3. Create and push a matching tag: `vX.Y.Z`.

The release workflow verifies the tag matches `Cargo.toml`, runs strict checks (`fmt`, `clippy -D warnings`, `test`, package dry-run), publishes to crates.io using the `CARGO_REGISTRY_TOKEN` GitHub secret, and uploads platform binaries to GitHub Releases.

## License

MIT
