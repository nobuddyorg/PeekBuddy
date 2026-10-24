#!/usr/bin/env bash
# SessionStart hook for Claude Code cloud sessions (#86): installs what
# `cargo xtask check` needs. Idempotent; a warm container only runs the
# `command -v` checks. Runs only in cloud sessions.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}"

# Toolchain, components and both Windows targets from rust-toolchain.toml.
rustup toolchain install
rustup show >/dev/null

# Versions match CI (ci.yml): bump them together.
install_tool() {
  local binary="$1" crate="$2" version="$3"
  if ! command -v "$binary" >/dev/null 2>&1; then
    cargo install --locked "$crate" --version "=$version"
  fi
}
install_tool cargo-nextest cargo-nextest 0.9.149
install_tool prek prek 0.5.7
install_tool cargo-xwin cargo-xwin 0.23.1
install_tool typos typos-cli 1.51.1

# Keep the MSVC CRT and Windows SDK that cargo-xwin downloads between
# sessions. Using them means accepting Microsoft's license (the owner accepted
# it once for this project, #86).
XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-$HOME/.cache/cargo-xwin}"
mkdir -p "$XWIN_CACHE_DIR"
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo "export XWIN_CACHE_DIR=\"$XWIN_CACHE_DIR\"" >>"$CLAUDE_ENV_FILE"
fi

# Build the xtask once so the first `cargo xtask check` starts fast.
cargo build --package xtask --quiet
