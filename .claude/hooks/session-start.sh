#!/bin/bash
# AlertU SessionStart hook.
#
# Prepares a Claude Code on the web session so `cargo test`, `cargo clippy`, and
# `cargo fmt` work immediately: ensures the clippy/rustfmt components exist and
# warms the dependency + build caches (which the container snapshots after this
# hook completes, so later sessions start fast).
#
# Synchronous and idempotent. Web-only: on a local machine it exits early.
set -euo pipefail

# Only run in the remote (Claude Code on the web) environment.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(pwd)}"

# Make cargo/rustc available if the toolchain lives under $HOME/.cargo.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

echo "[alertu hook] toolchain: $(cargo --version 2>/dev/null || echo 'cargo not found')"

# Ensure the linters are installed (no-op if already present).
if command -v rustup >/dev/null 2>&1; then
  rustup component add clippy rustfmt >/dev/null 2>&1 || true
fi

# Warm the dependency cache (safe to re-run; uses the committed Cargo.lock).
echo "[alertu hook] fetching dependencies..."
cargo fetch --locked || cargo fetch

# Warm the build cache for all crates and test targets so the first
# test/clippy run in the session is fast.
echo "[alertu hook] pre-building workspace (incl. tests)..."
cargo build --workspace --all-targets

echo "[alertu hook] ready: cargo test / cargo clippy / cargo fmt are good to go."
