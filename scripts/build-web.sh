#!/usr/bin/env bash
# Build the BearCAD web app (wasm32) into web/dist/.
#
# Needs: the wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`)
# and wasm-bindgen-cli matching the wasm-bindgen crate version in Cargo.lock
# (`cargo install wasm-bindgen-cli --version <version>`).
#
# The web build ships two Emscripten-built wasm modules alongside the app: the OCCT
# geometry kernel (web/kernel/, from scripts/build-occt-wasm.sh) and the Lua interpreter
# (web/lua/, from scripts/build-lua-wasm.sh). SQLite stays native-only; documents save/load
# as JSON through the browser's file pickers.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WANT_BINDGEN="$(grep -A2 'name = "wasm-bindgen"' Cargo.lock | grep version | head -1 | sed 's/.*"\(.*\)".*/\1/')"
HAVE_BINDGEN="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)"
if [[ "$HAVE_BINDGEN" != "$WANT_BINDGEN" ]]; then
  echo "installing wasm-bindgen-cli $WANT_BINDGEN (found: ${HAVE_BINDGEN:-none})..."
  cargo install wasm-bindgen-cli --version "$WANT_BINDGEN" --locked
fi

if [[ ! -f web/kernel/kernel.js ]]; then
  echo "==> kernel module missing; building OCCT for wasm (scripts/build-occt-wasm.sh)"
  scripts/build-occt-wasm.sh
fi

if [[ ! -f web/lua/lua.js ]]; then
  echo "==> Lua module missing; building Lua for wasm (scripts/build-lua-wasm.sh)"
  scripts/build-lua-wasm.sh
fi

echo "==> cargo build (wasm32, release, occt kernel via JS bridge)"
cargo build --release --target wasm32-unknown-unknown

echo "==> wasm-bindgen"
rm -rf web/dist
mkdir -p web/dist
wasm-bindgen target/wasm32-unknown-unknown/release/bearcad.wasm \
  --out-dir web/dist --out-name bearcad --target web --no-typescript

# Stamp the build into index.html so every asset URL carries it (#1049). wasm-bindgen
# mangles imported function names with a content hash, so a cached bearcad.js served
# alongside a fresh bearcad_bg.wasm fails to instantiate at all; versioning the URLs is what
# stops the browser expiring the two independently. The wasm's own hash is the stamp, so it
# changes exactly when the artifacts do and repeat builds stay byte-identical.
BUILD_STAMP="$(shasum -a 256 web/dist/bearcad_bg.wasm 2>/dev/null | cut -c1-12)"
if [[ -z "$BUILD_STAMP" ]]; then
  BUILD_STAMP="$(sha256sum web/dist/bearcad_bg.wasm | cut -c1-12)"
fi
sed "s/__BEARCAD_BUILD__/${BUILD_STAMP}/g" web/index.html > web/dist/index.html
echo "==> build stamp ${BUILD_STAMP}"
cp web/favicon.ico web/dist/ 2>/dev/null || true
cp web/kernel/kernel.js web/kernel/kernel.wasm web/dist/
cp web/lua/lua.js web/lua/lua.wasm web/dist/

echo
echo "Built web/dist/:"
ls -la web/dist/
echo
echo "Serve locally with e.g.:  python3 -m http.server -d web/dist 8080"
