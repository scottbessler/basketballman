#!/usr/bin/env bash
# Prepare a working basketballman dev environment: git hooks, crates, warm build.
# Safe to re-run. Used by humans, by `mise run setup`, and by the Claude SessionStart hook.
#
#   ./scripts/setup.sh              full setup
#   SKIP_BUILD=1 ./scripts/setup.sh skip the (slow) warm build
set -euo pipefail

cd "$(git rev-parse --show-toplevel 2>/dev/null || dirname "$(dirname "$0")")"

log() { printf '[setup] %s\n' "$*"; }
die() { printf '[setup] error: %s\n' "$*" >&2; exit 1; }

# --- toolchain ---------------------------------------------------------------
command -v cargo >/dev/null 2>&1 || die "cargo not found. Install Rust via https://rustup.rs (edition 2024 needs 1.85+)."
log "cargo $(cargo --version | awk '{print $2}'), rustc $(rustc --version | awk '{print $2}')"

for component in fmt clippy; do
  cargo "$component" --version >/dev/null 2>&1 ||
    die "cargo $component missing. Run: rustup component add $([ "$component" = fmt ] && echo rustfmt || echo clippy)"
done

# --- git hooks ---------------------------------------------------------------
# .githooks/pre-commit runs fmt+clippy; pre-push runs the full gate on main.
if git rev-parse --git-dir >/dev/null 2>&1; then
  git config core.hooksPath .githooks
  log "git hooks -> .githooks"
fi

# --- rust build --------------------------------------------------------------
log "fetching crates"
cargo fetch --locked

if [ "${SKIP_BUILD:-0}" = "1" ]; then
  log "SKIP_BUILD=1; skipping warm build"
else
  # Builds lib, bins and test targets so the first `cargo test` is fast.
  log "building (debug, with test targets) — first run takes a few minutes"
  cargo build --locked --tests
fi

cat <<'EOM'

[setup] done. Common commands:
  cargo test --locked            unit + invariant tests
  cargo run                      serve on :3000 (PASSKEY_DISABLED=1 to skip passkeys)
  ./dev.sh                       cargo run with file-watch restart
  cargo fmt && cargo clippy --all-targets
  mise run check                 everything CI runs, if you have mise
EOM
