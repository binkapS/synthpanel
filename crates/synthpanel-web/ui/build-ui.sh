#!/usr/bin/env bash
# Build the Slint UI into ui/dist, ready for include_dir! in synthpanel-web.
#
# Requires the wasm32-unknown-unknown target and a wasm-bindgen CLI whose
# version matches the wasm-bindgen crate in ui/Cargo.lock. Set WASM_BINDGEN to
# override the binary; it is otherwise looked up on PATH.
set -euo pipefail

cd "$(dirname "$0")"

WASM_BINDGEN="${WASM_BINDGEN:-wasm-bindgen}"
if ! command -v "$WASM_BINDGEN" >/dev/null 2>&1; then
  echo "error: $WASM_BINDGEN was not found." >&2
  echo "Install it with: cargo install wasm-bindgen-cli --version 0.2.130" >&2
  exit 1
fi

cargo build --release --target wasm32-unknown-unknown

rm -rf dist/pkg
mkdir -p dist/pkg
"$WASM_BINDGEN" target/wasm32-unknown-unknown/release/synthpanel_ui.wasm \
  --target web \
  --out-dir dist/pkg \
  --out-name synthpanel_ui \
  --no-typescript

cp web/index.html dist/index.html
cp web/slint-loader.js dist/slint-loader.js
printf 'built %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > dist/.stamp
