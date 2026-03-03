# Release Runbook

This repository uses a tag-driven release pipeline that publishes:

- the crate to crates.io
- Tier-1 binaries to GitHub Releases

## One-Time Setup

1. Create a crates.io API token with publish permission for this crate.
2. Add repository secret `CARGO_REGISTRY_TOKEN` in GitHub.
3. Protect `main` and require CI checks from `.github/workflows/ci.yml`.

## Normal Release Flow

1. Bump `[package].version` in `Cargo.toml`.
2. Merge to `main`.
3. Create and push a matching tag:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

The release workflow will:

1. Verify the tag matches `vX.Y.Z`.
2. Verify `Cargo.toml` version matches the tag version.
3. Run strict quality gates (`fmt`, `clippy -D warnings`, `test`, `cargo package --locked --no-verify`).
4. Build release binaries for:
   - `x86_64-unknown-linux-gnu`
   - `x86_64-apple-darwin`
   - `aarch64-apple-darwin`
   - `x86_64-pc-windows-msvc`
5. Publish to crates.io using `CARGO_REGISTRY_TOKEN`.
6. Create a GitHub Release and upload artifacts:
   - `acp-ws-bridge-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
   - `acp-ws-bridge-vX.Y.Z-x86_64-apple-darwin.tar.gz`
   - `acp-ws-bridge-vX.Y.Z-aarch64-apple-darwin.tar.gz`
   - `acp-ws-bridge-vX.Y.Z-x86_64-pc-windows-msvc.zip`

## Manual Release Workflow Dispatch

If needed, run `.github/workflows/release.yml` manually and provide an existing tag (`vX.Y.Z`) in the `tag` input.

The workflow still enforces version matching and quality gates before publish.

## Failure Behavior

- If tag/version verification fails, release stops before any publish step.
- If crates.io publish fails, GitHub Release creation does not run.
- Re-running failed jobs is safe if the tag and crate version are unchanged.
