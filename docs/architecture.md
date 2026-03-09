# Architecture

`acp-ws-bridge` is a transport bridge that relays ACP JSON-RPC messages between GitHub Copilot CLI and a remote client without rewriting payloads.

## High-level flow

```text
Remote client <-> WebSocket server <-> session relay <-> Copilot CLI
                    |
                    +-> REST API (health, sessions, history, usage)
```

The bridge exposes two network surfaces:

- a WebSocket endpoint for ACP traffic
- a REST API for health, session visibility, and local usage/history metadata

## Runtime modes

### Stdio mode (default)

- Each WebSocket client gets its own `copilot --acp --stdio --resume` child process.
- This is the default mode and is the simplest deployment model.
- The bridge owns the Copilot CLI stdin/stdout pipes directly.
- A custom exact command override can replace the default spawned command when configured.

### TCP mode

- The bridge connects to a Copilot CLI instance that listens on a TCP port.
- The bridge can also auto-spawn the CLI in TCP mode when configured to do so.
- This mode is useful when a shared Copilot CLI instance is preferred.
- With a custom exact command override, the user is responsible for making the spawned command match the configured TCP host/port behavior.

## Main components

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Parses config, starts logging, bootstraps REST + WebSocket services |
| `src/config.rs` | Defines CLI flags, command override support, and Copilot data-dir selection |
| `src/bridge.rs` | Wires HTTP/WebSocket routes and shared TLS handling |
| `src/ws.rs` | Relays ACP messages, handles keepalive, tracks activity |
| `src/session.rs` | Tracks in-memory session state, counters, idle timeout, cached commands |
| `src/copilot.rs` | Spawns Copilot CLI processes and detects CLI version |
| `src/api.rs` | Exposes `/health`, session APIs, history, usage, and Copilot metadata |
| `src/history.rs` | Reads historical session data from the configured Copilot data directory |
| `src/stats_cache.rs` | Caches usage data and stores bridge cache data under the configured Copilot data directory |
| `src/tls.rs` | Loads PEM TLS config with rustls and generates self-signed certs |
| `src/acp.rs` | ACP framing and JSON-RPC helpers |

## Session lifecycle

1. A client connects to the WebSocket endpoint.
2. The bridge registers a new session in `SessionManager`.
3. The session relay starts in stdio or TCP mode.
4. Message activity updates prompt/message counters and last-activity timestamps.
5. Cached command metadata from ACP updates is stored for the REST API.
6. If the session goes idle beyond the configured timeout, it is disconnected and cleaned up.

## Operational behaviors

### Keepalive

- The WebSocket relay sends periodic pings.
- Idle or dead peers are disconnected after timeout windows in `src/ws.rs`.

### Shared TLS

- When `--tls-cert` and `--tls-key` are provided, the WebSocket server and REST API share the same TLS configuration.

### Failure handling

- Failure to spawn Copilot CLI is logged, but the bridge can still start if an external CLI instance is available.
- Failure to bind the REST API is non-fatal; the WebSocket bridge can continue running.

## Design constraints

The bridge is intentionally conservative:

- preserve transparent relay behavior
- avoid mutating ACP payloads
- keep transport concerns separate from history/metrics concerns
- prefer explicit operational visibility via the REST API and logs
- execute custom ACP command overrides directly without shell evaluation

## Safe extension points

When extending the project, prefer changes that fit one of these categories:

- transport/runtime configuration
- observability and operational visibility
- deployment and contributor documentation
- testing and validation coverage

Changes that modify ACP payload semantics should be treated with extra caution.
