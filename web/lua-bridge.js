// Bridge between the BearCAD app module (wasm32-unknown-unknown, wasm-bindgen) and the Lua
// interpreter module (Lua 5.4, compiled separately with Emscripten — see
// scripts/build-lua-wasm.sh). The hosting page loads the Lua module first and stores its
// instance on `globalThis.bearcadLua`; if it failed to load, `lua_available()` is false and
// the app reports that scripts can't run — the same graceful story as the geometry kernel.
//
// The app calls `lua_run(src)` to execute a script. While it runs, each `bearcad.*` call the
// script makes crosses back to the app through `globalThis.bearcadDispatch` (installed by the
// Rust side, src/web_lua.rs) — the Lua module's C shim calls it via EM_JS. `lua_run` returns
// null on success, or the Lua error message string on failure.

function M() {
  return globalThis.bearcadLua || null;
}

export function lua_available() {
  return !!M();
}

export function lua_run(src) {
  const m = M();
  if (!m) return "Lua interpreter module not loaded";
  const srcPtr = m.stringToNewUTF8 ? m.stringToNewUTF8(src) : allocUtf8(m, src);
  // Error buffer for the message the shim writes back on failure.
  const errLen = 4096;
  const errPtr = m._malloc(errLen);
  m.HEAPU8[errPtr] = 0;
  const rc = m._bearcad_lua_run(srcPtr, errPtr, errLen);
  let err = null;
  if (rc !== 0) err = m.UTF8ToString(errPtr) || ("Lua error (code " + rc + ")");
  m._free(srcPtr);
  m._free(errPtr);
  return err;
}

// Fallback when stringToNewUTF8 isn't exported: encode + copy manually.
function allocUtf8(m, str) {
  const bytes = new TextEncoder().encode(str);
  const ptr = m._malloc(bytes.length + 1);
  m.HEAPU8.set(bytes, ptr);
  m.HEAPU8[ptr + bytes.length] = 0;
  return ptr;
}
