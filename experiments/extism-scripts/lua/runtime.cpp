#include <stdlib.h>
#include <string.h>
#define EXTISM_USE_LIBC
#include "extism-pdk.h"
extern "C" {
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"
}
#include "json_lua.h"

// LLVM 23 requires a definition of the tag referenced by Lua's tiny EH shim.
// Same tag layout as LLVM's libunwind/src/Unwind-wasm.c, without a second runtime.
__asm__(".globl __cpp_exception\n.tagtype __cpp_exception i32\n__cpp_exception:\n");

EXTISM_IMPORT_USER("zhang_now") extern uint64_t host_now(void);
EXTISM_IMPORT_USER("zhang_read_file") extern uint64_t host_read_file(uint64_t);
EXTISM_IMPORT_USER("zhang_query") extern uint64_t host_query(uint64_t);
EXTISM_IMPORT_USER("zhang_emit_error") extern void host_emit_error(uint64_t);

static int push_handle(lua_State *L, ExtismHandle handle) {
  size_t length = 0;
  char *text = extism_load_sz_dup(handle, &length);
  if (!text) return luaL_error(L, "cannot read host result");
  lua_pushlstring(L, text, strlen(text));
  free(text);
  extism_free(handle);
  return 1;
}

static int now(lua_State *L) { return push_handle(L, host_now()); }

static int call_text(lua_State *L, uint64_t (*function)(uint64_t)) {
  size_t length;
  const char *text = luaL_checklstring(L, 1, &length);
  ExtismHandle input = extism_alloc_buf(text, length);
  ExtismHandle result = function(input);
  extism_free(input);
  return push_handle(L, result);
}

static int query(lua_State *L) { return call_text(L, host_query); }
static int read_file(lua_State *L) { return call_text(L, host_read_file); }

static int emit_error(lua_State *L) {
  size_t length;
  const char *text = luaL_checklstring(L, 1, &length);
  ExtismHandle input = extism_alloc_buf(text, length);
  host_emit_error(input);
  extism_free(input);
  return 0;
}

static int config(lua_State *L) {
  ExtismHandle key = extism_alloc_buf_from_sz(luaL_checkstring(L, 1));
  ExtismHandle value = extism_config_get(key);
  extism_free(key);
  if (!value) { lua_pushnil(L); return 1; }
  return push_handle(L, value);
}

static const char *DISPATCH = R"lua(
local codec = _json
local json = {null = codec.null, encode = codec.encode}
json.decode = function(text)
  local value, position, err = codec.decode(text, 1, codec.null)
  if err then error(err) end
  if text:sub(position):find("%S") then error("trailing JSON data") end
  return value
end
local function unwrap(result)
  if result.Err then error(result.Err.message) end
  return result.Ok
end
local zhang = {
  now = function() return unwrap(json.decode(_host_now())) end,
  query = function(bql) return unwrap(json.decode(_host_query(bql))) end,
  emit_error = function(message) _host_emit_error(json.encode({message = message})) end,
}
local argument = json.decode(_input)
local source, filename, entry
if _entry == "run" then
  source, filename, entry = argument.source, argument.filename or "<script>", "run"
  argument = argument.input
elseif _entry == "process" then
  filename, entry = _config("script"), "process"
  local file = unwrap(json.decode(_host_read_file(filename)))
  assert(file.encoding == "utf8", "script must be UTF-8")
  source = file.content
else
  source, filename, entry = _config("script_source"), "<router-script>", "router"
end
local environment = setmetatable({zhang = zhang, json = json}, {__index = _G})
local module = assert(load(source, "@" .. filename, "t", environment))()
local fn = module[entry]
if not fn and entry == "process" then return json.encode(argument) end
assert(type(fn) == "function", "script does not define " .. entry)
return json.encode(fn(argument))
)lua";

static int failure(lua_State *L) {
  const char *message = lua_tostring(L, -1);
  extism_error_set_buf_from_sz(message ? message : "Lua execution failed");
  lua_close(L);
  return 1;
}

static int dispatch(const char *entry) {
  lua_State *L = luaL_newstate();
  if (!L) { extism_error_set_buf_from_sz("cannot allocate Lua state"); return 1; }
  luaL_openlibs(L);
  if (luaL_loadbuffer(L, JSON_LUA, sizeof(JSON_LUA) - 1, "@json.lua") || lua_pcall(L, 0, 1, 0)) return failure(L);
  lua_setglobal(L, "_json");
  lua_pushcfunction(L, now); lua_setglobal(L, "_host_now");
  lua_pushcfunction(L, query); lua_setglobal(L, "_host_query");
  lua_pushcfunction(L, read_file); lua_setglobal(L, "_host_read_file");
  lua_pushcfunction(L, emit_error); lua_setglobal(L, "_host_emit_error");
  lua_pushcfunction(L, config); lua_setglobal(L, "_config");
  lua_pushstring(L, entry); lua_setglobal(L, "_entry");
  char *input = extism_load_input_sz_dup(NULL);
  if (!input) { lua_pushliteral(L, "cannot read input"); return failure(L); }
  lua_pushstring(L, input); lua_setglobal(L, "_input");
  free(input);
  if (luaL_loadstring(L, DISPATCH) || lua_pcall(L, 0, 1, 0)) return failure(L);
  size_t length;
  const char *output = lua_tolstring(L, -1, &length);
  extism_output_buf(output, length);
  lua_close(L);
  return 0;
}

extern "C" {
int32_t EXTISM_EXPORTED_FUNCTION(run) { return dispatch("run"); }
int32_t EXTISM_EXPORTED_FUNCTION(processor) { return dispatch("process"); }
int32_t EXTISM_EXPORTED_FUNCTION(router) { return dispatch("router"); }
int32_t EXTISM_EXPORTED_FUNCTION(name) { extism_output_buf_from_sz("\"lua-script-runtime\""); return 0; }
int32_t EXTISM_EXPORTED_FUNCTION(version) { extism_output_buf_from_sz("\"0.0.1\""); return 0; }
int32_t EXTISM_EXPORTED_FUNCTION(supported_type) { extism_output_buf_from_sz("[\"Processor\",\"Router\"]"); return 0; }
}
