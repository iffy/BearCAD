#!/usr/bin/env bash
# Build the Lua interpreter for the BROWSER as a second Emscripten module (todoer #207).
#
# The web app is wasm32-unknown-unknown (eframe/wasm-bindgen) and can't link Lua's C, so —
# exactly like the OCCT geometry kernel (scripts/build-occt-wasm.sh) — the Lua interpreter
# ships as its own wasm module. The app runs a script via `bearcad_lua_run` through a small
# JS bridge (web/lua-bridge.js, src/web_lua.rs); each bearcad.* call the script makes crosses
# back to the app through globalThis.bearcadDispatch. Lua sources are vendored in
# third_party/lua/ (Lua 5.4.7, MIT).
#
# Usage:
#   scripts/build-lua-wasm.sh          # needs emcc (emscripten) on PATH
#
# Outputs:
#   web/lua/lua.js, web/lua/lua.wasm   the linked Lua module (ES module, EXPORT_NAME BearcadLua)

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lua_src="$repo_root/third_party/lua"
shim="$repo_root/cpp/bearcad_lua.cpp"

command -v emcc >/dev/null 2>&1 || { echo "error: emcc (emscripten) not found on PATH" >&2; exit 1; }

if [ ! -f "$lua_src/lua.h" ]; then
  echo "error: vendored Lua sources missing at $lua_src" >&2
  exit 1
fi

echo ">> Linking the BearCAD Lua module (lua.js/lua.wasm) ..."
mkdir -p "$repo_root/web/lua"

# The one entry point the app bridge calls, plus malloc/free for passing the script source
# and reading back the error string through the module heap.
exports='["_bearcad_lua_run","_malloc","_free"]'

# Every Lua core/lib .c except the standalone interpreter (lua.c) and compiler (luac.c),
# which carry their own main(). lctype/llex/etc are pulled in transitively but listing the
# whole set keeps the link explicit and order-independent.
lua_objs=$(ls "$lua_src"/*.c | grep -Ev '/(lua|luac)\.c$')

emcc "$shim" $lua_objs \
  -I"$lua_src" \
  -O2 \
  -sMODULARIZE=1 -sEXPORT_ES6=1 -sEXPORT_NAME=BearcadLua \
  -sALLOW_MEMORY_GROWTH=1 \
  -sENVIRONMENT=web \
  -sEXPORTED_FUNCTIONS="$exports" \
  -sEXPORTED_RUNTIME_METHODS='["ccall","cwrap","UTF8ToString","stringToUTF8","stringToNewUTF8","lengthBytesUTF8","HEAPU8"]' \
  -o "$repo_root/web/lua/lua.js"

echo
echo "Built:"
ls -la "$repo_root/web/lua/"
