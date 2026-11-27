#!/usr/bin/env bash
# Build the Slint UI into ui/dist, ready for include_dir! in synthpanel-web.
#
# Requires the wasm32-unknown-unknown target and a wasm-bindgen CLI whose
# version matches the wasm-bindgen crate in ui/Cargo.lock. Set WASM_BINDGEN to
# override the binary; it is otherwise looked up on PATH.
#
# Binaryen's wasm-opt is OFF by default: with wasm-bindgen 0.2.130 it breaks
# the generated externref/function tables, so an optimized module builds but
# fails at boot with "WebAssembly.Table.set(): Argument 1 is invalid". Set
# WASM_OPT to a wasm-opt binary only if you have verified compatibility with
# your wasm-bindgen version.
set -euo pipefail

cd "$(dirname "$0")"

WASM_BINDGEN="${WASM_BINDGEN:-wasm-bindgen}"
WASM_OPT="${WASM_OPT:-}"
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

# Optimize the wasm-bindgen output, not the pre-bindgen artifact: wasm-bindgen
# reads the target_features section while generating the JS glue, so stripping
# or rewriting that artifact first would produce worse (or invalid) glue.
WASM_FILE=dist/pkg/synthpanel_ui_bg.wasm
if command -v "$WASM_OPT" >/dev/null 2>&1; then
  before=$(wc -c < "$WASM_FILE")
  tmp="$WASM_FILE.opt"
  # rustc/wasm-bindgen use post-MVP features; enable the ones they emit so
  # binaryen can validate and optimize the module.
  # NOTE: do NOT enable reference-types. wasm-bindgen's JS glue installs JS
  # functions into a function-typed table; binaryen with reference-types can
  # rewrite that table into a form the glue can no longer populate, causing
  # "WebAssembly.Table.set(): Argument 1 is invalid" at boot.
  if "$WASM_OPT" --enable-bulk-memory \
       --enable-nontrapping-float-to-int --enable-mutable-globals \
       --enable-sign-ext --strip-debug -Oz "$WASM_FILE" -o "$tmp"; then
    mv "$tmp" "$WASM_FILE"
    after=$(wc -c < "$WASM_FILE")
    awk -v before="$before" -v after="$after" \
      'BEGIN { printf "wasm-opt: %d -> %d bytes (%.1f%% smaller)\n", before, after, 100 * (before - after) / before }'
  else
    rm -f "$tmp"
    echo "warning: $WASM_OPT failed; keeping the unoptimized $WASM_FILE" >&2
  fi
else
  echo "info: WASM_OPT is unset; skipping the opt-in wasm-opt pass." >&2
fi

cp web/index.html dist/index.html
cp web/slint-loader.js dist/slint-loader.js
printf 'built %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > dist/.stamp
