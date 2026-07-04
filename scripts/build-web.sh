#!/usr/bin/env bash
# Build the BearCAD web app (wasm32) into web/dist/.
#
# Needs: the wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`)
# and wasm-bindgen-cli matching the wasm-bindgen crate version in Cargo.lock
# (`cargo install wasm-bindgen-cli --version <version>`).
#
# The web build is the lean (no OCCT kernel, no Lua/SQLite) configuration; documents
# save/load as JSON through the browser's file pickers.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WANT_BINDGEN="$(grep -A2 'name = "wasm-bindgen"' Cargo.lock | grep version | head -1 | sed 's/.*"\(.*\)".*/\1/')"
HAVE_BINDGEN="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)"
if [[ "$HAVE_BINDGEN" != "$WANT_BINDGEN" ]]; then
  echo "installing wasm-bindgen-cli $WANT_BINDGEN (found: ${HAVE_BINDGEN:-none})..."
  cargo install wasm-bindgen-cli --version "$WANT_BINDGEN" --locked
fi

echo "==> cargo build (wasm32, release, lean)"
cargo build --release --target wasm32-unknown-unknown --no-default-features

echo "==> wasm-bindgen"
rm -rf web/dist
mkdir -p web/dist
wasm-bindgen target/wasm32-unknown-unknown/release/bearcad.wasm \
  --out-dir web/dist --out-name bearcad --target web --no-typescript

cp web/index.html web/dist/
cp web/favicon.ico web/dist/ 2>/dev/null || true

echo
echo "Built web/dist/:"
ls -la web/dist/
echo
echo "Serve locally with e.g.:  python3 -m http.server -d web/dist 8080"
