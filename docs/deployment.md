# Deployment

This document covers practical deployment guidance for running `acp-ws-bridge` as a long-lived service.

## What the service exposes

By default the bridge starts:

- a WebSocket listener on `--ws-port` (default `8765`)
- a REST API on `--api-port` (default `ws-port + 1`, usually `8766`)

The REST API is intended for local operational visibility, not as a public replacement for the WebSocket endpoint.

## Prerequisites

- GitHub Copilot CLI installed on the host
- Copilot CLI already authenticated for the runtime user
- access to the runtime user's `~/.copilot` directory for history/usage features
- TLS certificate and key if you plan to expose the bridge over untrusted networks

## Install options

### crates.io

```bash
cargo install acp-ws-bridge
```

### GitHub Releases

Download a prebuilt archive for your platform from the project's Releases page and place the binary on the host's `PATH`.

## Choosing a transport mode

### Default: stdio mode

Use stdio mode unless you have a specific reason to share a Copilot CLI instance.

```bash
acp-ws-bridge --ws-port 8765 --api-port 8766
```

### TCP mode

Use TCP mode when you want the bridge to connect to or auto-spawn a Copilot CLI instance listening on a port.

```bash
acp-ws-bridge \
  --ws-port 8765 \
  --api-port 8766 \
  --copilot-mode tcp \
  --copilot-port 3000
```

## TLS

To enable `wss://` and HTTPS for the REST API, provide both a certificate and key:

```bash
acp-ws-bridge \
  --ws-port 8765 \
  --api-port 8766 \
  --tls-cert /path/to/cert.pem \
  --tls-key /path/to/key.pem
```

For local testing you can generate a self-signed certificate:

```bash
acp-ws-bridge --generate-cert --tls-cert cert.pem --tls-key key.pem
```

## Example systemd unit

```ini
[Unit]
Description=acp-ws-bridge
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=bridge
WorkingDirectory=/opt/acp-ws-bridge
Environment=RUST_LOG=acp_ws_bridge=info
ExecStart=/usr/local/bin/acp-ws-bridge --ws-port 8765 --api-port 8766 --copilot-path /usr/local/bin/copilot
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Adjust paths, user, ports, and TLS flags to match your environment.

## Operational checks

### Health endpoint

```bash
curl http://127.0.0.1:8766/health
```

This should return bridge version, Copilot CLI version, and uptime.

### Session visibility

```bash
curl http://127.0.0.1:8766/api/sessions
curl http://127.0.0.1:8766/api/stats
```

### Logs

Use `RUST_LOG` or `--log-level` for runtime visibility.

Example:

```bash
RUST_LOG=acp_ws_bridge=debug acp-ws-bridge --ws-port 8765
```

## Troubleshooting

### Copilot CLI fails to spawn

- confirm the configured `--copilot-path` exists
- confirm the runtime user can run `copilot --version`
- confirm the runtime user is already authenticated with Copilot CLI

### REST API is unavailable

- check whether `--api-port` conflicts with another service
- remember the bridge can continue running even if the REST API fails to bind

### No history or usage data appears

- confirm the runtime user has a populated `~/.copilot` directory
- confirm `~/.copilot/session-store.db` exists for history endpoints
- confirm session-state event files exist for cached usage metrics

### TLS handshake issues

- verify the certificate matches the hostname clients use
- confirm both `--tls-cert` and `--tls-key` are provided
- test locally with the generated cert before exposing the service publicly
