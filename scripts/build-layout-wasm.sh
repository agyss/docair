#!/usr/bin/env bash
# Build docair-layout's move of a part for the page (crates/docair-layout-wasm/layout.wasm) and
# write the code version it was built from. Run it after changing docair-layout or
# docair-layout-wasm; `cargo test -p docair-layout-wasm` says when it is due.
set -euo pipefail
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null
cargo build -p docair-layout-wasm --target wasm32-unknown-unknown --profile wasm
cp target/wasm32-unknown-unknown/wasm/docair_layout_wasm.wasm crates/docair-layout-wasm/layout.wasm
cargo test -q -p docair-layout-wasm -- --ignored --exact tests::write_the_version
ls -l crates/docair-layout-wasm/layout.wasm
