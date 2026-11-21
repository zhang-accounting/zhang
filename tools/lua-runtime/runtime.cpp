// The Lua interpreter zhang pairs with every `plugin "x.lua"` directive: an Extism plugin exporting the plugin
// ABI v1 identity functions, `capabilities` and `router`, which run the script the host hands over in the config key
// `zhang.script` (named `zhang.script_name` in messages). Lua 5.4.8 (lua.wasm's onelua.c, compiled as C++ so its
// error handling rides wasm exception handling) and the JSON codec of json.cpp are linked in; this file is the glue.
//
// Sections: imports, memory budget, bridges, prelude (the `zhang`/`json` API, the sandbox, dispatch), the
// `zhang.decimal` library, exports.
//
// Every call gets a fresh Lua state: no Lua value survives between requests or between plugins. The host
// functions are C closures the prelude receives as arguments and keeps as upvalues of the `zhang` table; they
// are never globals, so a script cannot reach them by name.

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define EXTISM_USE_LIBC
#include "extism-pdk.h"
extern "C" {
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"
}

// json.cpp: push the `json` table of scripts: `null`, `encode`, `decode`, `object` and `array`
int json_open(lua_State *L);

// LLVM 23 requires a definition of the exception tag lua.wasm's EH shim throws (onelua.c: tag 0,
// `__cpp_exception`). Same tag layout as LLVM's libunwind, without a second runtime.
__asm__(".globl __cpp_exception\n.tagtype __cpp_exception i32\n__cpp_exception:\n");

// ---- imports: the host functions of plugin ABI v1 (zhang-shared/src/plugin_abi.rs, `import`) ------------------
// Importing one makes the host link it into this instance; zhang links every one of these into every plugin
// instance, and outside a router call the router ones answer `{"Err": {"kind": "unavailable"}}`.

EXTISM_IMPORT_USER("zhang_now") extern uint64_t host_now(void);
EXTISM_IMPORT_USER("zhang_ledger_info") extern uint64_t host_ledger_info(void);
EXTISM_IMPORT_USER("zhang_query") extern uint64_t host_query(uint64_t);
EXTISM_IMPORT_USER("zhang_query_params") extern uint64_t host_query_params(uint64_t);
EXTISM_IMPORT_USER("zhang_query_schema") extern uint64_t host_query_schema(void);
EXTISM_IMPORT_USER("zhang_query_builtin") extern uint64_t host_query_builtin(uint64_t);
EXTISM_IMPORT_USER("zhang_read_file") extern uint64_t host_read_file(uint64_t);
EXTISM_IMPORT_USER("zhang_list_dir") extern uint64_t host_list_dir(uint64_t);
EXTISM_IMPORT_USER("zhang_emit_error") extern void host_emit_error(uint64_t);
// the write capability: surfaced as `zhang.write.preview`/`zhang.write.commit` in the prelude, where the float lint
// on the batch lives, for a script that declares `writes` in its module table (the `capabilities` export below)
EXTISM_IMPORT_USER("zhang_write_preview") extern uint64_t host_write_preview(uint64_t);
EXTISM_IMPORT_USER("zhang_write_commit") extern uint64_t host_write_commit(uint64_t);

// ---- memory budget --------------------------------------------------------------------------------------------
// Lua allocates through this function only. A call may hold at most MEMORY_BUDGET bytes of Lua objects; an
// allocation taking it over fails, which Lua reports as a memory error ("not enough memory"), and the dispatcher
// turns that into "plugin `x` exceeded 64 MiB" instead of letting the plugin grow until the host runs out.

static const size_t MEMORY_BUDGET = 64u << 20;

struct Budget {
  size_t used;
  bool exceeded;
};

static void *budget_alloc(void *ud, void *ptr, size_t osize, size_t nsize) {
  Budget *budget = (Budget *)ud;
  // Lua's contract: without `ptr`, `osize` is the kind of object being created, not a size
  size_t old = ptr ? osize : 0;
  if (nsize == 0) {
    free(ptr);
    budget->used -= old;
    return NULL;
  }
  if (nsize > old && budget->used + (nsize - old) > MEMORY_BUDGET) {
    budget->exceeded = true;
    return NULL;
  }
  void *next = realloc(ptr, nsize);
  if (!next) return NULL;
  budget->used = budget->used - old + nsize;
  // Lua retries a failed allocation after a full collection: a growth that succeeds after that means the
  // budget was not exceeded after all, so a later unrelated error is not reported as the budget. Only a
  // growth clears the flag: unwinding from the memory error shrinks the Lua stack (a realloc to a smaller
  // size), and that must not hide the budget message
  if (nsize > old) budget->exceeded = false;
  return next;
}

// ---- bridges: the raw host functions as Lua C functions, length-aware both ways ------------------------------

// A block of the host's memory, freed when the scope holding it ends, however it ends. Lua raises its errors as
// C++ exceptions in this build, so a Lua error unwinding the scope (the budget refusing to copy a large answer
// into Lua) frees the block too: a block the bridge did not free would stay in the host's memory, outside the
// budget, for the rest of the call, and a script catching the error in a loop could pile them up into gigabytes
// (zhang-core/tests/plugins/host_answer_refused.lua).
struct HostBlock {
  ExtismHandle handle;
  explicit HostBlock(ExtismHandle handle) : handle(handle) {}
  HostBlock(const HostBlock &) = delete;
  HostBlock &operator=(const HostBlock &) = delete;
  ~HostBlock() {
    if (handle) extism_free(handle);
  }
};

// push the bytes of the host's block `handle` as a Lua string, with their real length (a NUL inside survives).
// The block is freed whether the copy fits the budget or not
static int push_handle(lua_State *L, ExtismHandle handle) {
  if (!handle) return luaL_error(L, "the host returned no memory block");
  HostBlock block(handle);
  uint64_t length = extism_length(handle);
  luaL_Buffer buffer;
  char *dest = luaL_buffinitsize(L, &buffer, (size_t)length);
  extism_load_from_handle(handle, 0, dest, (size_t)length);
  luaL_pushresultsize(&buffer, (size_t)length);
  return 1;
}

// call the host function `function` with the string argument 1 as a block and push its answer
static int call_with_text(lua_State *L, uint64_t (*function)(uint64_t)) {
  size_t length;
  const char *text = luaL_checklstring(L, 1, &length);
  ExtismHandle input = extism_alloc_buf(text, length);
  ExtismHandle result = function(input);
  extism_free(input);
  return push_handle(L, result);
}

static int bridge_now(lua_State *L) { return push_handle(L, host_now()); }
static int bridge_ledger_info(lua_State *L) { return push_handle(L, host_ledger_info()); }
static int bridge_query_schema(lua_State *L) { return push_handle(L, host_query_schema()); }
static int bridge_query_builtin(lua_State *L) { return call_with_text(L, host_query_builtin); }
static int bridge_query(lua_State *L) { return call_with_text(L, host_query); }
static int bridge_query_params(lua_State *L) { return call_with_text(L, host_query_params); }
static int bridge_read_file(lua_State *L) { return call_with_text(L, host_read_file); }
static int bridge_list_dir(lua_State *L) { return call_with_text(L, host_list_dir); }
static int bridge_write_preview(lua_State *L) { return call_with_text(L, host_write_preview); }
static int bridge_write_commit(lua_State *L) { return call_with_text(L, host_write_commit); }

static int bridge_emit_error(lua_State *L) {
  size_t length;
  const char *payload = luaL_checklstring(L, 1, &length);
  ExtismHandle input = extism_alloc_buf(payload, length);
  host_emit_error(input);
  extism_free(input);
  return 0;
}

// write argument 1 to the host's log at the info level, where `zhang serve` shows it
static int bridge_log(lua_State *L) {
  size_t length;
  const char *text = luaL_checklstring(L, 1, &length);
  extism_log(text, length, ExtismLogInfo);
  return 0;
}

// the config value of the key in argument 1, or nil: the flat config every plugin gets
static int bridge_config(lua_State *L) {
  size_t length;
  const char *key = luaL_checklstring(L, 1, &length);
  ExtismHandle name = extism_alloc_buf(key, length);
  ExtismHandle value = extism_config_get(name);
  extism_free(name);
  if (!value) {
    lua_pushnil(L);
    return 1;
  }
  return push_handle(L, value);
}

static const luaL_Reg BRIDGES[] = {
    {"now", bridge_now},
    {"ledger_info", bridge_ledger_info},
    {"query", bridge_query},
    {"query_params", bridge_query_params},
    {"query_schema", bridge_query_schema},
    {"query_builtin", bridge_query_builtin},
    {"read_file", bridge_read_file},
    {"list_dir", bridge_list_dir},
    {"write_preview", bridge_write_preview},
    {"write_commit", bridge_write_commit},
    {"emit_error", bridge_emit_error},
    {"config", bridge_config},
    {"log", bridge_log},
    {NULL, NULL},
};

// ---- prelude: the `zhang` and `json` API, the sandbox, and the dispatch of one export ------------------------
// Runs once per call with the raw bridges, the JSON codec, the global table, the script and the entry point as its
// arguments, and returns `true, output` (the JSON the export answers) or `false, message` (the error the export
// reports). A runtime error raised by the script is caught with a traceback; a syntax error or a module table of
// the wrong shape is reported without one.

static const char PRELUDE[] = R"lua(
local raw, codec, base, source, script_name, entry, input, decimal_source = ...
local type, error, pairs, ipairs, next, tostring, tonumber, select, setmetatable, getmetatable, rawget, rawset, rawequal, xpcall, pcall =
      type, error, pairs, ipairs, next, tostring, tonumber, select, setmetatable, getmetatable, rawget, rawset, rawequal, xpcall, pcall
local sbyte, schar, sgsub, ssub, slower, sformat = string.byte, string.char, string.gsub, string.sub, string.lower, string.format
local concat, utf8len = table.concat, utf8.len
local rawload, traceback = base.load, base.debug.traceback

-- ---- json: the C codec of json.cpp (null, encode, decode, and the sealed object/array markers) --------------------
local json = codec

-- ---- base64, for bytes that are not UTF-8: file contents from the host, response bodies to it -----------------
local ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local B64, B64REV = {}, {}
for i = 0, 63 do
  B64[i] = ssub(ALPHABET, i + 1, i + 1)
  B64REV[sbyte(ALPHABET, i + 1)] = i
end

local function base64_encode(data)
  local out, part, n = {}, {}, #data
  local full = n - n % 3
  for i = 1, full, 3 do
    local a, b, c = sbyte(data, i, i + 2)
    local v = (a << 16) | (b << 8) | c
    part[#part + 1] = B64[(v >> 18) & 63] .. B64[(v >> 12) & 63] .. B64[(v >> 6) & 63] .. B64[v & 63]
    if #part == 4096 then out[#out + 1] = concat(part); part = {} end
  end
  local rest = n - full
  if rest == 1 then
    local v = sbyte(data, n) << 16
    part[#part + 1] = B64[(v >> 18) & 63] .. B64[(v >> 12) & 63] .. "=="
  elseif rest == 2 then
    local a, b = sbyte(data, n - 1, n)
    local v = (a << 16) | (b << 8)
    part[#part + 1] = B64[(v >> 18) & 63] .. B64[(v >> 12) & 63] .. B64[(v >> 6) & 63] .. "="
  end
  out[#out + 1] = concat(part)
  return concat(out)
end

local function base64_decode(text)
  text = sgsub(text, "[^%w%+/]", "")
  local out, part, n = {}, {}, #text
  for i = 1, n, 4 do
    local a, b, c, d = sbyte(text, i, i + 3)
    local v = (B64REV[a] << 18) | ((b and B64REV[b] or 0) << 12) | ((c and B64REV[c] or 0) << 6) | (d and B64REV[d] or 0)
    if d then part[#part + 1] = schar((v >> 16) & 255, (v >> 8) & 255, v & 255)
    elseif c then part[#part + 1] = schar((v >> 16) & 255, (v >> 8) & 255)
    elseif b then part[#part + 1] = schar((v >> 16) & 255)
    end
    if #part == 4096 then out[#out + 1] = concat(part); part = {} end
  end
  out[#out + 1] = concat(part)
  return concat(out)
end

-- ---- the host's answers: `{"Ok": value}` is the value, `{"Err": {kind, message, line?, column?}}` raises ------
local function unwrap(text)
  local result = json.decode(text)
  local err = type(result) == "table" and result.Err
  if err then
    local line, column = err.line, err.column
    -- level 3: the script's line, past this function and the zhang.* function that called it
    if err.kind == "query" and type(line) == "number" and type(column) == "number" then
      error(sformat("query error at line %d, column %d: %s", line, column, tostring(err.message)), 3)
    elseif err.kind == "query" then
      error("query error: " .. tostring(err.message), 3)
    end
    error(tostring(err.kind) .. ": " .. tostring(err.message), 3)
  end
  return result.Ok
end

-- ---- the zhang table --------------------------------------------------------------------------------------------
local function constructor(kind)
  return function(value)
    if value == nil then value = json.null end
    return {type = kind, value = value}
  end
end

local zhang = {
  -- the version of this API, raised by every release that adds to the `zhang` table (the guide lists what each
  -- version added), so a script checks `zhang.api_version >= n` before it uses what version n brought. 1 is the API
  -- of the first release with Lua scripts, zhang.decimal values included
  api_version = 1,
  str = constructor("str"),
  int = constructor("int"),
  bool = constructor("bool"),
  date = constructor("date"),
}

-- ---- zhang.decimal: exact decimal values (RFC #698 §4) ------------------------------------------------------------
-- The library is a chunk of its own (DECIMAL_LIBRARY), read the first time a script calls zhang.decimal or reads one
-- of its functions. A value it made has a metatable whose __metatable is "zhang.decimal"
local DECIMAL_SEAL = "zhang.decimal"
local decimal_library
local function decimal()
  if decimal_library == nil then
    local chunk, err = rawload(decimal_source, "=zhang.decimal", "t", base)
    if not chunk then error(err) end
    decimal_library = chunk(json)
  end
  return decimal_library
end
zhang.decimal = setmetatable({}, {
  -- a call makes a value; `make` raises at the line of the script calling this function
  __call = function(_, value)
    local made = decimal().make(value)
    return made
  end,
  -- a function of the library, kept in the table once read
  __index = function(table, key)
    local found = decimal()[key]
    if key ~= "make" and found ~= nil then rawset(table, key, found) else found = nil end
    return found
  end,
  __metatable = "zhang.decimal library",
})

function zhang.set(value)
  if value == nil then value = json.null
  elseif type(value) == "table" and getmetatable(value) == nil then value = json.array(value)
  end
  return {type = "set", value = value}
end

function zhang.now()
  local value = unwrap(raw.now())
  return value
end

function zhang.ledger_info()
  local value = unwrap(raw.ledger_info())
  return value
end

function zhang.query(bql)
  if type(bql) ~= "string" then error("zhang.query expects the query text as a string, got " .. type(bql), 2) end
  local value = unwrap(raw.query(bql))
  return value
end

function zhang.query_params(bql, params)
  if type(bql) ~= "string" then error("zhang.query_params expects the query text as a string, got " .. type(bql), 2) end
  local bound = json.object()
  if params ~= nil then
    if type(params) ~= "table" then error("zhang.query_params expects the parameters as a table, got " .. type(params), 2) end
    for name, param in pairs(params) do
      if type(param) ~= "table" or type(param.type) ~= "string" then
        error("parameter :" .. tostring(name) .. " must be made with zhang.str/int/decimal/bool/date/set or be a {type = ..., value = ...} table", 2)
      end
      -- a decimal value crosses as the typed parameter it is; json.encode alone would write its text
      if getmetatable(param) == DECIMAL_SEAL then param = {type = "decimal", value = rawget(param, "value")} end
      bound[name] = param
    end
  end
  local value = unwrap(raw.query_params(json.encode({bql = bql, params = bound})))
  return value
end

-- one of the server's built-in queries, by name, with its parameters as plain values: json.null is NULL, a decimal
-- is a string ("12.50"), a date "YYYY-MM-DD", a set a list of strings. A float is refused before anything is sent
function zhang.query_builtin(name, params)
  if type(name) ~= "string" then error("zhang.query_builtin expects the query's name as a string, got " .. type(name), 2) end
  local bound = json.object()
  if params ~= nil then
    if type(params) ~= "table" then error("zhang.query_builtin expects the parameters as a table, got " .. type(params), 2) end
    for key, param in pairs(params) do
      if math.type(param) == "float" then
        error(sformat("parameter :%s is the float %s: give a decimal as a string, such as \"%s\"", tostring(key), tostring(param), tostring(param)), 2)
      end
      if type(param) == "table" and getmetatable(param) == nil then param = json.array(param) end
      bound[key] = param
    end
  end
  local value = unwrap(raw.query_builtin(json.encode({name = name, params = bound})))
  return value
end

function zhang.query_schema()
  local value = unwrap(raw.query_schema())
  return value
end

-- the bytes of a granted file as a Lua string; the host sends what is not UTF-8 as base64
function zhang.read_file(path)
  local file = unwrap(raw.read_file(path))
  if file.encoding == "base64" then return base64_decode(file.content) end
  return file.content
end

-- the entries of a granted directory: `{{name = ..., kind = "file" | "dir"}, ...}`, sorted by name
function zhang.list_dir(path)
  local dir = unwrap(raw.list_dir(path))
  return dir.entries
end

-- report a problem in the ledger (a processor's tool; in a router it is only logged), with optional metas
function zhang.emit_error(message, metas)
  local payload = {message = tostring(message)}
  if type(metas) == "table" then
    local copy = json.object()
    for key, value in pairs(metas) do copy[tostring(key)] = tostring(value) end
    payload.metas = copy
  end
  raw.emit_error(json.encode(payload))
end

-- a config value, the flat config a wasm plugin gets: ledger options, the directive's metas, `zhang.*`
function zhang.config(key)
  if type(key) ~= "string" then error("zhang.config expects the key as a string, got " .. type(key), 2) end
  return raw.config(key)
end

-- ---- writes: zhang.write.preview(batch) and zhang.write.commit(batch, token) ----------------------------------
-- A batch is a list of directive tables, each with a `kind` ("transaction" or "open") and the fields of that kind;
-- amounts are strings. A non-integer float anywhere in a batch is refused before the batch is encoded, so a float
-- never reaches the ledger, not even rounded; the host refuses every JSON number besides. The host's answer is the
-- preview or the commit as a table, and its `Err` raises `kind: message`, as every zhang.* function's does.
local function first_float(value, path)
  local kind = type(value)
  if kind == "number" then
    if math.type(value) == "float" then return path, value end
  elseif kind == "table" then
    for key, item in pairs(value) do
      local at, found = first_float(item, path .. (type(key) == "number" and ("[" .. key .. "]") or ("." .. tostring(key))))
      if at then return at, found end
    end
  end
  return nil
end

local function write_input(name, batch, token)
  if type(batch) ~= "table" then error(name .. " expects the batch as a list of directive tables, got " .. type(batch), 3) end
  local at, float = first_float(batch, "directives")
  if at then error(sformat("amount `%s` must be a string (%s)", tostring(float), at), 3) end
  local directives = json.array()
  for index, directive in ipairs(batch) do directives[index] = directive end
  local input = json.object({directives = directives})
  if token ~= nil then input.token = token end
  return json.encode(input)
end

zhang.write = {}

function zhang.write.preview(batch)
  local value = unwrap(raw.write_preview(write_input("zhang.write.preview", batch)))
  return value
end

function zhang.write.commit(batch, token)
  if type(token) ~= "string" then error("zhang.write.commit expects the token as a string, got " .. type(token), 2) end
  local value = unwrap(raw.write_commit(write_input("zhang.write.commit", batch, token)))
  return value
end

-- ---- the sandbox: a fresh environment over the read-only base; load is text-only ------------------------------
-- the environment's metatable is sealed too: `getmetatable(_ENV)` cannot hand the base back
local env = setmetatable({}, {__index = base, __metatable = "sandbox"})
env._G = env
env.zhang = zhang
env.json = json

-- `load` reads text chunks only (a binary chunk is refused as if the mode were "t") and runs them in the
-- script's environment unless one is given
local function sandboxed_load(chunk, name, mode, ...)
  if select("#", ...) == 0 then return rawload(chunk, name, "t", env) end
  return rawload(chunk, name, "t", (...))
end
rawset(base, "load", sandboxed_load)

-- `print` has no stdout to reach: its line goes to the host's log at the info level, prefixed with the
-- script's name, so a plugin author sees it in the `zhang serve` log
local function sandboxed_print(...)
  local parts = {}
  for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
  raw.log(script_name .. ": " .. concat(parts, "\t"))
end
rawset(base, "print", sandboxed_print)
setmetatable(base, {
  __newindex = function(_, key) error("the sandbox is read-only: cannot set " .. tostring(key), 2) end,
  __metatable = "sandbox",
})

-- ---- dispatch -----------------------------------------------------------------------------------------------------
local function fail(message)
  if type(message) ~= "string" then message = tostring(message) end
  return false, message
end

local chunk, err = rawload(source, "@" .. script_name, "t", env)
if not chunk then return fail(err) end
local ok, module = xpcall(chunk, traceback)
if not ok then return fail(module) end
if type(module) ~= "table" then
  return fail(script_name .. ": the script must return a module table {name = ..., version = ..., types = {\"Router\"}, router = function(request) ... end}, got " .. type(module))
end

if entry == "name" then
  if type(module.name) ~= "string" or module.name == "" then return fail(script_name .. ": the module table needs a non-empty string `name`") end
  return true, json.encode(module.name)
elseif entry == "version" then
  if type(module.version) ~= "string" then return fail(script_name .. ": the module table needs a string `version`") end
  return true, json.encode(module.version)
elseif entry == "supported_type" then
  local types = module.types
  if types == nil then types = {"Router"} end
  if type(types) ~= "table" then return fail(script_name .. ": `types` must be a list such as {\"Router\"}, got " .. type(types)) end
  local entries = 0
  for _ in pairs(types) do entries = entries + 1 end
  if entries ~= #types then return fail(script_name .. ": `types` must be a list such as {\"Router\"}, got a table with keys that are not 1, 2, ...") end
  local declared = json.array()
  for _, kind in ipairs(types) do
    if kind == "Processor" or kind == "Mapper" then
      return fail(script_name .. ": Lua processors and mappers are not supported yet (#698 P1): zhang.decimal adds, subtracts and compares but does not multiply or divide, and the host would take back an amount turned into a JSON number")
    elseif kind ~= "Router" then
      return fail(sformat('%s: unknown plugin type %q; Lua scripts declare types = {"Router"}', script_name, tostring(kind)))
    end
    declared[#declared + 1] = kind
  end
  return true, json.encode(declared)
elseif entry == "capabilities" then
  -- the kinds the script writes, `writes = {"transaction", "open"}` in its module table; nothing without it
  local writes = module.writes
  local declared = json.array()
  if writes ~= nil then
    if type(writes) ~= "table" then return fail(script_name .. ": `writes` must be a list such as {\"transaction\", \"open\"}, got " .. type(writes)) end
    for _, kind in ipairs(writes) do
      if type(kind) ~= "string" then return fail(script_name .. ": `writes` must be a list of strings such as {\"transaction\", \"open\"}, got " .. type(kind)) end
      declared[#declared + 1] = kind
    end
  end
  return true, json.encode({writes = declared})
elseif entry == "router" then
  local handler = module.router
  if type(handler) ~= "function" then return fail(script_name .. ": the module table has no `router` function") end
  local request = json.decode(input)
  local ok, response = xpcall(handler, traceback, request)
  if not ok then return fail(response) end
  -- anything but a table is passed on as it is, so the host refuses it as an invalid response
  if type(response) ~= "table" then
    local encoded, text = pcall(json.encode, response)
    return true, encoded and text or "null"
  end
  local wire = json.object()
  local status = response.status
  if math.type(status) == "float" and math.tointeger(status) then status = math.tointeger(status) end
  wire.status = status
  local headers, has_content_type = json.object(), false
  if type(response.headers) == "table" then
    for name, value in pairs(response.headers) do
      if type(name) == "string" and slower(name) == "content-type" then has_content_type = true end
      headers[name] = type(value) == "number" and tostring(value) or value
    end
    wire.headers = headers
  elseif response.headers ~= nil then
    wire.headers = response.headers
  else
    wire.headers = headers
  end
  if response.json ~= nil then
    wire.body = json.encode(response.json)
    wire.body_encoding = "utf8"
    if not has_content_type then headers["content-type"] = "application/json; charset=utf-8" end
  elseif response.body ~= nil then
    local body = response.body
    if type(body) == "string" and response.body_encoding == nil and utf8len(body) == nil then
      wire.body, wire.body_encoding = base64_encode(body), "base64"
    else
      wire.body, wire.body_encoding = body, response.body_encoding
    end
  end
  return true, json.encode(wire)
end
return fail("unknown entry point " .. tostring(entry))
)lua";

// ---- zhang.decimal: the library, a chunk the prelude reads the first time a script uses it ------------------------

static const char DECIMAL_LIBRARY[] = R"lua(
-- zhang.decimal, read by the prelude the first time a script uses it (a call that does not, a router answering a
-- request with queries, say, does not pay for reading it): the prelude runs this chunk with the json table, and gets
-- the library's functions back, and `make`, the constructor.
--
-- A decimal value is the table {type = "decimal", value = text} the constructor has always made, now with the sealed
-- metatable DECIMAL. `text` is the number in plain notation as zhang writes one (`plain_decimal` of zhang-shared): an
-- optional "-", the digits, and "." with the fractional digits when its scale is not 0; never "-0". It is still a
-- query parameter, json.encode writes it as its text (json.cpp reads `__jsontype = "decimal"`), and it adds,
-- subtracts and compares exactly, in Lua over its digits: a sum or a difference keeps the larger scale of the two, as
-- BigDecimal's `a + b` and `&a - &b` do, and a comparison is by value. There is no multiplication and no division:
-- those stay in the host (queries, zhang.convert). A number of more than MAX_DIGITS digits is refused, as zhang_convert
-- refuses one.
--
-- Next to `type` and `value`, a value holds its parsed form, which the arithmetic reads: `neg`, `scale` (its digits
-- after the point), and either `small`, its signed coefficient when that has at most SMALL digits, or `mag`, its
-- digits without a leading zero (`mag` of a small one is added when the long arithmetic needs it). A value never
-- changes once made, so the value of a text is made once and kept, up to 4096 texts at a time.
--
-- Errors name the script's line: a function given `level` raises at that level counted from itself, and never makes
-- a tail call on the way, which would take a level off the stack.
local json = ...
local type, error, tostring, tonumber, setmetatable, getmetatable, rawget, rawequal =
      type, error, tostring, tonumber, setmetatable, getmetatable, rawget, rawequal
local sformat, ssub, srep, sfind, smatch, concat, mtype =
      string.format, string.sub, string.rep, string.find, string.match, table.concat, math.type
local MAX_DIGITS = 1000
-- a coefficient of at most SMALL digits adds in a Lua integer (a sum of two stays below 2^63); longer ones are added
-- SMALL digits at a time
local SMALL = 18
local POW10, PAD, POINT, NEGATIVE_POINT, FIT = {[0] = 1}, {}, {}, {}, {}
for i = 1, SMALL do
  POW10[i] = POW10[i - 1] * 10
  PAD[i] = "%0" .. i .. "d"
  POINT[i] = "%d.%0" .. i .. "d"
  NEGATIVE_POINT[i] = "-%d.%0" .. i .. "d"
end
-- a coefficient smaller than FIT[k] in magnitude still has at most SMALL digits with k zeros after it
for k = 0, SMALL do FIT[k] = POW10[SMALL - k] end

local DECIMAL_SEAL = "zhang.decimal"
local DECIMAL = {__jsontype = "decimal", __metatable = DECIMAL_SEAL, __name = "zhang.decimal"}

-- the text of a decimal in plain notation, from its digits
local function plain(neg, mag, scale)
  local text = mag
  if scale > 0 then
    local n = #mag
    if n > scale then text = ssub(mag, 1, n - scale) .. "." .. ssub(mag, n - scale + 1)
    else text = "0." .. srep("0", scale - n) .. mag end
  end
  if neg then return "-" .. text end
  return text
end

-- the value of the signed coefficient `v` at `scale`, |v| < 2 * 10^SMALL
local function small_value(v, scale)
  local neg = v < 0
  local magnitude = neg and -v or v
  local text
  if scale == 0 then
    text = sformat("%d", v)
  elseif scale <= SMALL then
    local unit = POW10[scale]
    text = sformat(neg and NEGATIVE_POINT[scale] or POINT[scale], magnitude // unit, magnitude % unit)
  else
    local digits = sformat("%d", magnitude)
    text = (neg and "-0." or "0.") .. srep("0", scale - #digits) .. digits
  end
  if magnitude >= POW10[SMALL] then
    return setmetatable({type = "decimal", value = text, neg = neg, scale = scale, mag = sformat("%d", magnitude)}, DECIMAL)
  end
  return setmetatable({type = "decimal", value = text, neg = neg, scale = scale, small = v}, DECIMAL)
end

-- the value of the digits `mag` (no leading zero) at `scale`, negative when `neg` and not zero
local function long_value(neg, mag, scale)
  if #mag <= SMALL then
    local v = tonumber(mag)
    if neg then v = -v end
    local made = small_value(v, scale)
    return made
  end
  return setmetatable({type = "decimal", value = plain(neg, mag, scale), neg = neg, scale = scale, mag = mag}, DECIMAL)
end

local ZERO = small_value(0, 0)

-- the digits of a value, without a leading zero
local function mag_of(value)
  local mag = value.mag
  if not mag then
    local small = value.small
    mag = sformat("%d", small < 0 and -small or small)
    value.mag = mag
  end
  return mag
end

local function shown(text)
  if #text > 40 then return sformat("%q… (%d bytes)", ssub(text, 1, 40), #text) end
  return sformat("%q", text)
end

-- the values of the texts read so far
local parsed, parsed_count = {}, 0

-- the value of `text`: an optional sign, digits, and a point with digits, at least one digit in all
local function parse(text, level)
  local found = parsed[text]
  if found then return found end
  if #text > MAX_DIGITS + 2 then error(sformat("decimal %s has more than %d digits", shown(text), MAX_DIGITS), level) end
  local sign, int, fraction = smatch(text, "^([+-]?)(%d*)%.?(%d*)$")
  if not sign or #int + #fraction == 0 then
    error(sformat("decimal must be a number in plain notation, such as \"-12.50\", got %s", shown(text)), level)
  end
  if #int + #fraction > MAX_DIGITS then error(sformat("decimal %s has more than %d digits", shown(text), MAX_DIGITS), level) end
  local digits = int .. fraction
  local first = sfind(digits, "[1-9]")
  local value = long_value(sign == "-" and first ~= nil, first and ssub(digits, first) or "0", #fraction)
  if parsed_count >= 4096 then parsed, parsed_count = {}, 0 end
  parsed[text] = value
  parsed_count = parsed_count + 1
  return value
end

local function is_decimal(value)
  return type(value) == "table" and getmetatable(value) == DECIMAL_SEAL
end

-- the value of an operand: a decimal value, a string in plain notation, or an integer. A float is refused, as the
-- read-side twin of the write batch's lint, so a rounded number never reaches a sum or a query
local function operand(value, level)
  local kind, found = type(value), nil
  if kind == "table" then
    if getmetatable(value) == DECIMAL_SEAL then return value end
    if rawget(value, "type") == "decimal" then error("a NULL decimal has no value to compute with", level) end
    error("decimal must be a string or an integer, got table", level)
  elseif kind == "string" then
    found = parse(value, level + 1)
  elseif mtype(value) == "integer" then
    found = parse(sformat("%d", value), level + 1)
  elseif kind == "number" then
    error("decimal must be a string or an integer, got " .. tostring(value), level)
  else
    error("decimal must be a string or an integer, got " .. kind, level)
  end
  return found
end

local function trim(digits)
  local first = sfind(digits, "[1-9]")
  if first then return ssub(digits, first) end
  return "0"
end

-- the digits of a + b (`sign` 1) or a - b (`sign` -1, a >= b), two strings of digits of the same length, SMALL digits
-- at a time from the right
local function combine(a, b, sign)
  local parts, carry, stop = {}, 0, #a
  for at = (stop + SMALL - 1) // SMALL, 1, -1 do
    local start = stop - SMALL + 1
    if start < 1 then start = 1 end
    local width = stop - start + 1
    local part = tonumber(ssub(a, start, stop)) + sign * (tonumber(ssub(b, start, stop)) + carry)
    if part >= POW10[width] then part, carry = part - POW10[width], 1
    elseif part < 0 then part, carry = part + POW10[width], 1
    else carry = 0 end
    parts[at] = sformat(PAD[width], part)
    stop = start - 1
  end
  local digits = concat(parts)
  if carry == 1 then return "1" .. digits end
  return digits
end

-- the digits of a value at a scale at least its own
local function digits_at(value, scale)
  local mag = mag_of(value)
  if scale == value.scale or mag == "0" then return mag end
  return mag .. srep("0", scale - value.scale)
end

-- a + b, or a - b when `subtract`, over their digits
local function long_add(a, b, subtract)
  local scale = a.scale
  if b.scale > scale then scale = b.scale end
  local ma, mb = digits_at(a, scale), digits_at(b, scale)
  local bneg = b.neg
  if subtract and mb ~= "0" then bneg = not bneg end
  if #ma < #mb then ma = srep("0", #mb - #ma) .. ma elseif #mb < #ma then mb = srep("0", #ma - #mb) .. mb end
  if a.neg == bneg then return long_value(a.neg, trim(combine(ma, mb, 1)), scale) end
  if ma == mb then return long_value(false, "0", scale) end
  if ma > mb then return long_value(a.neg, trim(combine(ma, mb, -1)), scale) end
  return long_value(bneg, trim(combine(mb, ma, -1)), scale)
end

-- a + b, or a - b when `subtract`: exact, at the larger scale of the two
local function add(a, b, subtract)
  local x, y, sa, sb = a.small, b.small, a.scale, b.scale
  if x and y then
    if sa == sb then
      if subtract then return small_value(x - y, sa) end
      return small_value(x + y, sa)
    end
    local scale, da, db = sa, 0, 0
    if sa < sb then scale, da = sb, sb - sa else db = sa - sb end
    local fa, fb = FIT[da], FIT[db]
    if fa and fb and x < fa and x > -fa and y < fb and y > -fb then
      x, y = x * POW10[da], y * POW10[db]
      if subtract then return small_value(x - y, scale) end
      return small_value(x + y, scale)
    end
  end
  return long_add(a, b, subtract)
end

-- -1, 0 or 1 as a is less than, equal to or greater than b, by value
local function compare(a, b)
  local an = a.neg
  if an ~= b.neg then return an and -1 or 1 end
  local x, y, sa, sb = a.small, b.small, a.scale, b.scale
  if x and y then
    if sa ~= sb then
      local da, db = 0, 0
      if sa < sb then da = sb - sa else db = sa - sb end
      local fa, fb = FIT[da], FIT[db]
      if fa and fb and x < fa and x > -fa and y < fb and y > -fb then x, y = x * POW10[da], y * POW10[db] else x = nil end
    end
    if x then
      if x < y then return -1 elseif x > y then return 1 end
      return 0
    end
  end
  local scale = sa
  if sb > scale then scale = sb end
  local ma, mb = digits_at(a, scale), digits_at(b, scale)
  if ma == mb then return 0 end
  local order = 1
  if #ma < #mb or (#ma == #mb and ma < mb) then order = -1 end
  if an then return -order end
  return order
end

local function no_arithmetic(what)
  return function()
    error("zhang.decimal has no " .. what .. ": compute it in the host, with a query or zhang.convert", 2)
  end
end

local METHODS = {}
function METHODS.neg(a)
  local value = operand(a, 3)
  local small = value.small
  if small then return small_value(-small, value.scale) end
  return long_value(not value.neg, value.mag, value.scale)
end
function METHODS.abs(a)
  local value = operand(a, 3)
  local small = value.small
  if small then return small_value(small < 0 and -small or small, value.scale) end
  return long_value(false, value.mag, value.scale)
end
function METHODS.is_zero(a) return operand(a, 3).small == 0 end
function METHODS.sign(a)
  local value = operand(a, 3)
  if value.small == 0 then return 0 end
  return value.neg and -1 or 1
end
function METHODS.cmp(a, b) return compare(operand(a, 3), operand(b, 3)) end

DECIMAL.__index = METHODS
DECIMAL.__add = function(a, b) return add(operand(a, 3), operand(b, 3), false) end
DECIMAL.__sub = function(a, b) return add(operand(a, 3), operand(b, 3), true) end
DECIMAL.__unm = METHODS.neg
-- Lua asks __eq only when both sides are tables: a decimal value equals another by value, and no other table, such as
-- json.null
DECIMAL.__eq = function(a, b)
  if not (is_decimal(a) and is_decimal(b)) then return false end
  return compare(a, b) == 0
end
DECIMAL.__lt = function(a, b) return compare(operand(a, 3), operand(b, 3)) < 0 end
DECIMAL.__le = function(a, b) return compare(operand(a, 3), operand(b, 3)) <= 0 end
DECIMAL.__mul = no_arithmetic("multiplication")
DECIMAL.__div = no_arithmetic("division")
DECIMAL.__idiv = no_arithmetic("division")
DECIMAL.__mod = no_arithmetic("remainder")
DECIMAL.__pow = no_arithmetic("power")
DECIMAL.__tostring = function(a) return rawget(a, "value") end
DECIMAL.__concat = function(a, b)
  if is_decimal(a) then a = rawget(a, "value") end
  if is_decimal(b) then b = rawget(b, "value") end
  return a .. b
end

-- zhang.decimal(value): the decimal value of a string in plain notation, an integer or a decimal value; nil (or
-- json.null) is the NULL parameter it always was. zhang.decimal.add, .sub, .neg, .abs, .cmp, .sign, .is_zero and .sum
-- take any operand a decimal value takes, strings included
local function make_decimal(value)
  if value == nil or rawequal(value, json.null) then return {type = "decimal", value = json.null} end
  local made = operand(value, 4)
  return made
end

return {
  make = make_decimal,
  add = DECIMAL.__add,
  sub = DECIMAL.__sub,
  neg = METHODS.neg,
  abs = METHODS.abs,
  cmp = METHODS.cmp,
  sign = METHODS.sign,
  is_zero = METHODS.is_zero,
  is_decimal = is_decimal,
  -- the sum of a list of operands, "0" for none
  sum = function(list)
    if type(list) ~= "table" then error("zhang.decimal.sum expects a list of decimals, got " .. type(list), 2) end
    local total = ZERO
    for index = 1, #list do total = add(total, operand(list[index], 3), false) end
    return total
  end,
}
)lua";

// ---- exports ----------------------------------------------------------------------------------------------------

// push the config value of `key`, or raise naming it: the host sets both keys for every script
static void push_required_config(lua_State *L, const char *key) {
  ExtismHandle value = extism_config_get_buf_from_sz(key);
  if (!value) luaL_error(L, "the Lua interpreter needs the config key %s: declare the script as plugin \"<path>.lua\"", key);
  push_handle(L, value);
}

// push the call's input with its real length
static void push_input(lua_State *L) {
  uint64_t length = extism_input_length();
  luaL_Buffer buffer;
  char *dest = luaL_buffinitsize(L, &buffer, (size_t)length);
  extism_load_input(0, dest, (size_t)length);
  luaL_pushresultsize(&buffer, (size_t)length);
}

// open the libraries a script may use: base without the file loaders, string, table, math, utf8, coroutine, and
// `debug` reduced to `traceback`. No io, os, package or require.
static void open_sandbox(lua_State *L) {
  static const luaL_Reg libraries[] = {
      {LUA_GNAME, luaopen_base},     {LUA_STRLIBNAME, luaopen_string}, {LUA_TABLIBNAME, luaopen_table},
      {LUA_MATHLIBNAME, luaopen_math}, {LUA_UTF8LIBNAME, luaopen_utf8}, {LUA_COLIBNAME, luaopen_coroutine},
      {NULL, NULL},
  };
  for (const luaL_Reg *library = libraries; library->func; library++) {
    luaL_requiref(L, library->name, library->func, 1);
    lua_pop(L, 1);
  }
  lua_pushnil(L);
  lua_setglobal(L, "dofile");
  lua_pushnil(L);
  lua_setglobal(L, "loadfile");
  luaL_requiref(L, LUA_DBLIBNAME, luaopen_debug, 0);
  lua_getfield(L, -1, "traceback");
  lua_newtable(L);
  lua_insert(L, -2);
  lua_setfield(L, -2, "traceback");
  lua_setglobal(L, "debug");
  lua_pop(L, 1);
}

// everything a call does, in protected mode: argument 1 is the entry point's name. Returns what the prelude returns
static int run_protected(lua_State *L) {
  const char *entry = (const char *)lua_touserdata(L, 1);
  open_sandbox(L);
  json_open(L);
  int codec = lua_gettop(L);
  if (luaL_loadbufferx(L, PRELUDE, sizeof(PRELUDE) - 1, "=zhang", "t") != LUA_OK) lua_error(L);
  lua_newtable(L);
  luaL_setfuncs(L, BRIDGES, 0);
  lua_pushvalue(L, codec);
  lua_pushglobaltable(L);
  push_required_config(L, "zhang.script");
  push_required_config(L, "zhang.script_name");
  lua_pushstring(L, entry);
  push_input(L);
  lua_pushlstring(L, DECIMAL_LIBRARY, sizeof(DECIMAL_LIBRARY) - 1);
  lua_call(L, 8, 2);
  return 2;
}

// the name of the script for the budget message, read outside Lua so it is available after a memory error
static char *script_name_copy(void) {
  ExtismHandle value = extism_config_get_buf_from_sz("zhang.script_name");
  if (!value) return NULL;
  char *name = extism_load_sz_dup(value, NULL);
  extism_free(value);
  return name;
}

static int dispatch(const char *entry) {
  Budget budget = {0, false};
  lua_State *L = lua_newstate(budget_alloc, &budget);
  if (!L) {
    extism_error_set_buf_from_sz("cannot create a Lua state");
    return 1;
  }
  lua_pushcfunction(L, run_protected);
  lua_pushlightuserdata(L, (void *)entry);
  int status = lua_pcall(L, 1, 2, 0);
  int failed = status != LUA_OK || !lua_toboolean(L, -2);
  if (failed && budget.exceeded) {
    char *name = script_name_copy();
    char message[512];
    snprintf(message, sizeof message, "plugin `%s` exceeded 64 MiB", name ? name : "?");
    free(name);
    extism_error_set_buf_from_sz(message);
  } else if (failed) {
    size_t length;
    const char *message = lua_tolstring(L, -1, &length);
    if (message) extism_error_set_buf(message, length);
    else extism_error_set_buf_from_sz("the Lua script failed without a message");
  } else {
    size_t length;
    const char *output = lua_tolstring(L, -1, &length);
    extism_output_buf(output ? output : "null", output ? length : 4);
  }
  lua_close(L);
  return failed ? 1 : 0;
}

extern "C" {
int32_t EXTISM_EXPORTED_FUNCTION(name) { return dispatch("name"); }
int32_t EXTISM_EXPORTED_FUNCTION(version) { return dispatch("version"); }
int32_t EXTISM_EXPORTED_FUNCTION(supported_type) { return dispatch("supported_type"); }
int32_t EXTISM_EXPORTED_FUNCTION(capabilities) { return dispatch("capabilities"); }
int32_t EXTISM_EXPORTED_FUNCTION(router) { return dispatch("router"); }
}
