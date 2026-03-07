# Contributing to acp-ws-bridge

Thanks for taking the time to improve `acp-ws-bridge`.

This project is a small Rust bridge with a deliberately narrow scope: relay ACP JSON-RPC traffic between GitHub Copilot CLI and remote clients without rewriting messages. Contributions should preserve that design goal and keep operational behavior predictable.

## Before you start

- Read the [README](README.md) for setup and runtime behavior.
- Review the relevant docs in [`docs/`](docs/) before changing release, deployment, or testing behavior.
- Search existing issues and pull requests before starting overlapping work.

## Local setup

```bash
cargo build
cargo test --all-targets --all-features
```

If you are changing the behavior of the bridge, REST API, or release process, run the full validation suite before opening a pull request:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash scripts/validate_agents.sh
cargo package --locked --allow-dirty --no-verify
```

## Change expectations

Please keep changes focused and reviewable.

- Preserve the transparent relay behavior unless the change explicitly requires otherwise.
- Add or update tests when behavior changes.
- Update `README.md` or docs in `docs/` when flags, API behavior, deployment guidance, or release steps change.
- Add a short note to `CHANGELOG.md` under `Unreleased` for user-facing changes.
- Avoid unrelated refactors in the same pull request.

## Pull request checklist

Before opening a PR, make sure you can answer "yes" to these:

- [ ] The change has a clear problem statement and scope.
- [ ] I ran the relevant validation commands locally.
- [ ] I added or updated tests for behavior changes.
- [ ] I updated documentation for user-facing, operational, or release-facing changes.
- [ ] I described any limitations, follow-up work, or trade-offs in the PR body.

## Commit messages

Prefer concise prefixes that match the existing history, for example:

- `feat:`
- `fix:`
- `docs:`
- `chore:`
- `ci:`
- `refactor:`
- `test:`

## Review expectations

- Small, focused PRs are preferred over large batches of unrelated changes.
- Maintainers may request follow-up tests or docs before merge.
- Security-sensitive issues should follow the process in [SECURITY.md](SECURITY.md), not public issues.
