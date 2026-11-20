#!/usr/bin/env bash
# Runs once after the devcontainer is created.
set -euo pipefail

# The cargo registry volume is created root-owned on first mount.
sudo chown -R "$(id -u):$(id -g)" /usr/local/cargo/registry "${CLAUDE_CONFIG_DIR}" || true

rustup component add clippy rustfmt
# The page's WebAssembly module is built with docair (docair-layout-wasm's build.rs).
rustup target add wasm32-unknown-unknown

# beads issue tracker (https://github.com/gastownhall/beads)
npm install -g @beads/bd
bd --version

if [ -f Cargo.toml ]; then
  cargo fetch
fi

if [ ! -d .beads ]; then
  echo
  echo "beads is not initialised yet. Run once:  bd init"
fi
