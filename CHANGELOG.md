# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] — 2026-03-09

### Added

- `--acp-command` with `--command` alias for exact Copilot ACP spawn-command overrides, parsed without shell execution
- `--copilot-dir` to use a custom Copilot data directory for history, session-state, and bridge stats cache data

### Changed

- history and usage endpoints now resolve data from the configured Copilot data directory instead of always assuming `~/.copilot`

## [0.2.1] — 2026-03-07

### Added

- Contributor-facing repository docs: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and `SECURITY.md`
- GitHub issue and pull request templates for bugs, feature requests, and review checklists
- New architecture, deployment, and testing docs under `docs/`
- A dedicated coverage workflow and README badges for CI, coverage, crates.io, and license

## [0.2.0] — 2026-03-07

### Added

- **Copilot CLI 1.0 GA compatibility** — verified and documented support for Copilot CLI v1.0.x (GA since v0.0.418)
- **`COPILOT_CLI=1` environment variable** — set when spawning Copilot CLI processes so git hooks can detect CLI subprocesses (Copilot CLI v0.0.421+)
- **Copilot CLI version detection** — runs `copilot --version` at startup and includes `copilot_cli_version` in the `/health` endpoint response
- **`GET /api/copilot/info` endpoint** — returns CLI version, executable path, transport mode, GA status, and feature capabilities (reasoning_effort, mcp_elicitations, exit_plan_mode, session_metrics, v1_stable)
- `CHANGELOG.md` following Keep a Changelog format

### Notes

- All new ACP protocol methods introduced in Copilot CLI v0.0.418–v1.0.2 (exitPlanMode.request, MCP elicitations, reasoning effort config) are transparently relayed without bridge changes
- Session usage metrics from `events.jsonl` (Copilot CLI v0.0.422+) are already consumed by the existing StatsCache

## [0.1.2] — 2026-03-04

### Added

- HTTPS support for REST API (shares TLS config with WebSocket server)
- Copilot CLI usage statistics endpoint (`GET /api/copilot/usage`)
- Incremental `events.jsonl` ingestion via `StatsCache`

### Fixed

- Security improvements for TLS configuration

## [0.1.1] — 2026-03-03

### Added

- TLS certificate generation scripts (cross-platform, macOS/Linux, Windows)
- `--generate-cert` flag for built-in self-signed certificate generation
- `--cert-hostnames` flag for custom hostnames in generated certificates

## [0.1.0] — 2026-03-02

### Added

- Initial release
- WebSocket-to-stdio bridge for ACP JSON-RPC relay
- TCP and stdio transport modes for Copilot CLI
- REST API with health, sessions, stats, and history endpoints
- Session management with idle timeout and keepalive
- TLS/WSS support for secure connections
- Cross-platform release binaries (Linux x64, macOS x64/ARM64, Windows x64)
- CI/CD with tag-driven releases to crates.io and GitHub Releases

[Unreleased]: https://github.com/ytthuan/acp-ws-bridge/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/ytthuan/acp-ws-bridge/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/ytthuan/acp-ws-bridge/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ytthuan/acp-ws-bridge/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/ytthuan/acp-ws-bridge/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/ytthuan/acp-ws-bridge/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/ytthuan/acp-ws-bridge/releases/tag/v0.1.0
