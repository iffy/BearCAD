// BearCAD web Lua interpreter shim (todoer #179/#207).
//
// The web app is wasm32-unknown-unknown (eframe/wasm-bindgen) and can't link Lua's C
// directly, so — exactly like the OCCT geometry kernel (cpp/bearcad_kernel.cpp) — the Lua
// interpreter ships as a *second* Emscripten module (see scripts/build-lua-wasm.sh, output
// web/lua/lua.js + lua.wasm). The app runs a script by calling `bearcad_lua_run(src)`
// through the JS bridge (web/lua-bridge.js, src/web_lua.rs).
//
// Every `bearcad.<name>{...}` call the script makes is forwarded, as a (name, json-args)
// pair of strings, out through one hook — `__bearcad_call` — which crosses back to the Rust
// app module via the JS global `globalThis.bearcadDispatch(name, jsonArgs) -> jsonResult`.
// The Rust side (src/script_json.rs + src/web_lua.rs) turns that into the very same
// Instruction/Action the desktop mlua closures drive. So the whole `bearcad.*` API is
// realized here by a tiny data-driven prelude rather than ~130 hand-written bindings.

#include <cstdlib>
#include <cstring>
#include <string>

#include <emscripten.h>

extern "C" {
#include "lauxlib.h"
#include "lua.h"
#include "lualib.h"
}

// Cross from the Lua module back to the Rust app module. `globalThis.bearcadDispatch` is
// installed by the app before it runs a script (src/web_lua.rs); if it's missing (script
// run outside the app), every call degrades to an error result. The returned string is
// malloc'd in this module's heap and freed by the caller below.
EM_JS(char*, bearcad_js_dispatch, (const char* name, const char* args), {
  const dispatch = globalThis.bearcadDispatch;
  let res;
  if (typeof dispatch === "function") {
    try {
      res = dispatch(UTF8ToString(name), UTF8ToString(args));
    } catch (e) {
      res = JSON.stringify({ error: String(e) });
    }
  } else {
    res = JSON.stringify({ error: "bearcad dispatcher not connected" });
  }
  if (typeof res !== "string") res = JSON.stringify({ error: "dispatcher returned non-string" });
  const len = lengthBytesUTF8(res) + 1;
  const ptr = _malloc(len);
  stringToUTF8(res, ptr, len);
  return ptr;
});

// Lua C function `__bearcad_call(name, jsonArgs) -> jsonResult`. Both sides are strings; the
// prelude json-encodes the Lua arguments and json-decodes the result.
static int l_bearcad_call(lua_State* L) {
  const char* name = luaL_checkstring(L, 1);
  const char* args = luaL_optstring(L, 2, "{}");
  char* res = bearcad_js_dispatch(name, args);
  if (res) {
    lua_pushstring(L, res);
    free(res);
  } else {
    lua_pushliteral(L, "{\"error\":\"dispatch returned null\"}");
  }
  return 1;
}

// The Lua prelude: a compact JSON codec (after rxi/json.lua, MIT) plus the `bearcad` table.
// Any `bearcad.foo(...)` forwards `foo` and the JSON-encoded arguments to `__bearcad_call`:
// a single table argument is sent as the argument object; otherwise the positional
// arguments are sent under `__args` (the Rust side maps them to names). The decoded result
// is `{ok=true[, value=...]}` or `{error="..."}`; the latter raises a catchable Lua error.
static const char* BEARCAD_PRELUDE = R"LUA(
local json = {}
do
  local function kind_of(obj)
    if type(obj) ~= 'table' then return type(obj) end
    local i = 1
    for _ in pairs(obj) do
      if obj[i] ~= nil then i = i + 1 else return 'table' end
    end
    if i == 1 then return 'table' else return 'array' end
  end
  local escape_map = { ['"']='\\"', ['\\']='\\\\', ['\b']='\\b', ['\f']='\\f',
    ['\n']='\\n', ['\r']='\\r', ['\t']='\\t' }
  local function escape_char(c)
    return escape_map[c] or string.format('\\u%04x', c:byte())
  end
  local encode
  local function encode_nil() return 'null' end
  local function encode_table(val, stack)
    local res = {}
    stack = stack or {}
    if stack[val] then error('circular reference') end
    stack[val] = true
    if rawget(val, 1) ~= nil or next(val) == nil then
      local n = 0
      for k in pairs(val) do
        if type(k) ~= 'number' then error('invalid key type (array)') end
        n = n + 1
      end
      if n ~= #val then error('sparse array') end
      for _, v in ipairs(val) do res[#res+1] = encode(v, stack) end
      stack[val] = nil
      return '[' .. table.concat(res, ',') .. ']'
    else
      for k, v in pairs(val) do
        if type(k) ~= 'string' then error('invalid key type (object)') end
        res[#res+1] = encode(k, stack) .. ':' .. encode(v, stack)
      end
      stack[val] = nil
      return '{' .. table.concat(res, ',') .. '}'
    end
  end
  local function encode_string(val)
    return '"' .. val:gsub('[%z\1-\31\\"]', escape_char) .. '"'
  end
  local function encode_number(val)
    if val ~= val or val <= -math.huge or val >= math.huge then
      error('unexpected number value')
    end
    return string.format('%.14g', val)
  end
  encode = function(val, stack)
    local t = type(val)
    if t == 'nil' then return 'null'
    elseif t == 'table' then return encode_table(val, stack)
    elseif t == 'string' then return encode_string(val)
    elseif t == 'number' then return encode_number(val)
    elseif t == 'boolean' then return tostring(val)
    else error("unexpected type '" .. t .. "'") end
  end
  function json.encode(val) return encode(val) end

  local parse
  local function create_set(...)
    local res = {}
    for i = 1, select('#', ...) do res[select(i, ...)] = true end
    return res
  end
  local space_chars = create_set(' ', '\t', '\r', '\n')
  local delim_chars = create_set(' ', '\t', '\r', '\n', ']', '}', ',')
  local escape_chars = create_set('\\', '/', '"', 'b', 'f', 'n', 'r', 't', 'u')
  local literals = create_set('true', 'false', 'null')
  local literal_map = { ['true']=true, ['false']=false, ['null']=nil }
  local function next_char(str, idx, set, negate)
    for i = idx, #str do
      if set[str:sub(i, i)] ~= negate then return i end
    end
    return #str + 1
  end
  local function decode_error(str, idx, msg)
    error(msg .. ' at position ' .. idx)
  end
  local function parse_string(str, i)
    local res, j, k = '', i + 1, i + 1
    while j <= #str do
      local x = str:byte(j)
      if x < 32 then decode_error(str, j, 'control character in string')
      elseif x == 92 then -- backslash
        res = res .. str:sub(k, j - 1)
        j = j + 1
        local c = str:sub(j, j)
        if c == 'u' then
          local hex = str:sub(j + 1, j + 4)
          j = j + 4
          local n = tonumber(hex, 16) or decode_error(str, j, 'invalid unicode escape')
          if n < 0x80 then res = res .. string.char(n)
          elseif n < 0x800 then
            res = res .. string.char(0xC0 + math.floor(n / 0x40), 0x80 + n % 0x40)
          else
            res = res .. string.char(0xE0 + math.floor(n / 0x1000),
              0x80 + math.floor(n / 0x40) % 0x40, 0x80 + n % 0x40)
          end
        elseif escape_chars[c] then
          local map = { ['b']='\b', ['f']='\f', ['n']='\n', ['r']='\r', ['t']='\t' }
          res = res .. (map[c] or c)
        else decode_error(str, j, "invalid escape '\\" .. c .. "'") end
        k = j + 1
      elseif x == 34 then -- quote
        res = res .. str:sub(k, j - 1)
        return res, j + 1
      end
      j = j + 1
    end
    decode_error(str, i, 'expected closing quote')
  end
  local function parse_number(str, i)
    local x = next_char(str, i, delim_chars)
    local s = str:sub(i, x - 1)
    local n = tonumber(s) or decode_error(str, i, "invalid number '" .. s .. "'")
    return n, x
  end
  local function parse_literal(str, i)
    local x = next_char(str, i, delim_chars)
    local word = str:sub(i, x - 1)
    if not literals[word] then decode_error(str, i, "invalid literal '" .. word .. "'") end
    return literal_map[word], x
  end
  local function parse_array(str, i)
    local res, n = {}, 1
    i = i + 1
    while true do
      local x
      i = next_char(str, i, space_chars, true)
      if str:sub(i, i) == ']' then return res, i + 1 end
      x, i = parse(str, i)
      res[n] = x
      n = n + 1
      i = next_char(str, i, space_chars, true)
      local c = str:sub(i, i)
      i = i + 1
      if c == ']' then return res, i end
      if c ~= ',' then decode_error(str, i, "expected ']' or ','") end
    end
  end
  local function parse_object(str, i)
    local res = {}
    i = i + 1
    while true do
      local key, val
      i = next_char(str, i, space_chars, true)
      if str:sub(i, i) == '}' then return res, i + 1 end
      if str:sub(i, i) ~= '"' then decode_error(str, i, 'expected string key') end
      key, i = parse_string(str, i)
      i = next_char(str, i, space_chars, true)
      if str:sub(i, i) ~= ':' then decode_error(str, i, "expected ':'") end
      i = next_char(str, i + 1, space_chars, true)
      val, i = parse(str, i)
      res[key] = val
      i = next_char(str, i, space_chars, true)
      local c = str:sub(i, i)
      i = i + 1
      if c == '}' then return res, i end
      if c ~= ',' then decode_error(str, i, "expected '}' or ','") end
    end
  end
  local char_func_map = {
    ['"']=parse_string, ['0']=parse_number, ['1']=parse_number, ['2']=parse_number,
    ['3']=parse_number, ['4']=parse_number, ['5']=parse_number, ['6']=parse_number,
    ['7']=parse_number, ['8']=parse_number, ['9']=parse_number, ['-']=parse_number,
    ['t']=parse_literal, ['f']=parse_literal, ['n']=parse_literal,
    ['[']=parse_array, ['{']=parse_object,
  }
  parse = function(str, idx)
    local chr = str:sub(idx, idx)
    local f = char_func_map[chr]
    if f then return f(str, idx) end
    decode_error(str, idx, "unexpected character '" .. chr .. "'")
  end
  function json.decode(str)
    local res, idx = parse(str, next_char(str, 1, space_chars, true))
    idx = next_char(str, idx, space_chars, true)
    if idx <= #str then decode_error(str, idx, 'trailing garbage') end
    return res
  end
end

_G.json = json

local function call(name, ...)
  local n = select('#', ...)
  local payload
  if n == 1 and type((...)) == 'table' then
    payload = (...)
  elseif n == 0 then
    payload = {}
  else
    payload = { __args = { ... } }
  end
  local res = json.decode(__bearcad_call(name, json.encode(payload)))
  if type(res) == 'table' and res.error ~= nil then
    error(res.error, 2)
  end
  if type(res) == 'table' then return res.value end
  return res
end

bearcad = setmetatable({}, {
  __index = function(_, name)
    return function(...) return call(name, ...) end
  end,
})
-- `bearcad.ui.*` and `bearcad.fps.*` are the same flat verbs under grouping tables, matching
-- the desktop namespacing (e.g. bearcad.ui.orbit -> the "orbit" verb).
bearcad.ui = setmetatable({}, {
  __index = function(_, name)
    return function(...) return call(name, ...) end
  end,
})
)LUA";

// Run a Lua script. Returns 0 on success; on error returns non-zero and writes the message
// into `errbuf` (NUL-terminated, truncated to `errlen`).
extern "C" int bearcad_lua_run(const char* src, char* errbuf, int errlen) {
  auto fail = [&](const char* msg) {
    if (errbuf && errlen > 0) {
      std::strncpy(errbuf, msg ? msg : "unknown error", errlen - 1);
      errbuf[errlen - 1] = '\0';
    }
  };

  lua_State* L = luaL_newstate();
  if (!L) {
    fail("out of memory creating Lua state");
    return 1;
  }
  luaL_openlibs(L);
  lua_pushcfunction(L, l_bearcad_call);
  lua_setglobal(L, "__bearcad_call");

  if (luaL_dostring(L, BEARCAD_PRELUDE) != LUA_OK) {
    fail(lua_tostring(L, -1));
    lua_close(L);
    return 2;
  }
  if (luaL_dostring(L, src) != LUA_OK) {
    fail(lua_tostring(L, -1));
    lua_close(L);
    return 3;
  }
  lua_close(L);
  return 0;
}
