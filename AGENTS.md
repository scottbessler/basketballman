# Working in this repo

Rust (axum + askama) server-rendered basketball league manager. Backend in
`src/`, templates in `templates/` (compiled into the binary), static assets in
`static/` (served from disk), invariant tests in `tests/`. `SPEC.md` is the
source of truth for the game; `SIM.SPEC.md` covers the simulation engine.

## Setup

```sh
./scripts/setup.sh
```

Idempotent: enables the repo git hooks, fetches crates, and warms the build.
`SKIP_BUILD=1` skips the slow build. Claude Code web sessions run this
automatically via `.claude/hooks/session-start.sh`.

Toolchain: Rust 1.95 (edition 2024) — see `.mise.toml`; CI and the Dockerfile
pin the same version.

## Commands

| Task | Command |
| --- | --- |
| Tests | `cargo test --locked` |
| Lint | `cargo fmt --check && cargo clippy --locked --all-targets --all-features` |
| Run server | `cargo run` (:3000), or `./dev.sh` to restart on file change |
| Everything CI runs | `mise run check` |

## Notes

- `warnings = "deny"` and `clippy::all = "deny"` are set in `Cargo.toml`; a
  warning fails the build, so fix rather than `#[allow]`.
- Passkeys can't be driven headlessly. `dev.sh` sets `PASSKEY_DISABLED=1`; use
  it for any local run you need to sign into.
- League state persists to `$DATA_PATH/league.json` (default `data/`); delete
  it to regenerate.
- Pushing to `main` runs CI and then deploys to Fly.io (`basketballman.fly.dev`).
  The pre-push hook runs the full gate locally for pushes to `main`.
