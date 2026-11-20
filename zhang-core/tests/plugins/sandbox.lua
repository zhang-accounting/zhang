-- A router plugin that probes its sandbox and answers what it found, so a test sees that io, os,
-- package, require, dofile, loadfile, loadstring and debug beyond traceback are unavailable, that a
-- binary chunk does not load, that the raw host bridges and the interpreter's own locals are not
-- globals, and that the libraries a script may use are there.
local M = { name = "sandbox", version = "0.1.0" }

-- names that must not be reachable as globals: the raw bridges, the interpreter's locals, the spike's globals
local hidden = {
  "now", "query", "query_params", "query_schema", "ledger_info", "read_file", "list_dir", "emit_error", "config", "log",
  "raw", "codec", "base", "source", "script_name", "entry", "input", "unwrap", "env",
  "_host_now", "_host_query", "_host_read_file", "_host_emit_error", "_config", "_input", "_json", "_entry",
  "io", "os", "package", "require", "dofile", "loadfile", "loadstring", "module", "newproxy", "gcinfo",
}

function M.router(request)
  local leaked = json.array()
  for _, name in ipairs(hidden) do
    if _G[name] ~= nil or rawget(_G, name) ~= nil then leaked[#leaked + 1] = name end
  end
  local sandbox = getmetatable(_ENV).__index
  for _, name in ipairs(hidden) do
    if rawget(sandbox, name) ~= nil then leaked[#leaked + 1] = "sandbox." .. name end
  end
  local debug_names = json.array()
  for name in pairs(debug) do debug_names[#debug_names + 1] = name end
  local binary = string.dump(function() return 1 end)
  local binary_chunk, binary_error = load(binary)
  local explicit_binary_chunk = load(binary, "=dumped", "b")
  script_global = "set"
  local base_write_ok = pcall(function() sandbox.leak = 1 end)
  return { json = {
    leaked = leaked,
    debug_names = debug_names,
    binary_load_refused = binary_chunk == nil and type(binary_error) == "string",
    explicit_binary_load_refused = explicit_binary_chunk == nil,
    text_load_works = load("return 1 + 1")() == 2,
    loaded_chunk_sees_the_api = load("return zhang ~= nil and json ~= nil")(),
    loaded_chunk_sees_script_globals = load("return script_global")() == "set",
    global_stays_in_env = rawget(_ENV, "script_global") == "set" and rawget(sandbox, "script_global") == nil,
    sandbox_is_read_only = not base_write_ok and getmetatable(sandbox) == "sandbox",
    g_is_env = _G == _ENV,
    libraries = {
      string = type(string.format) == "function", table = type(table.concat) == "function",
      math = type(math.floor) == "function", utf8 = type(utf8.len) == "function",
      coroutine = type(coroutine.wrap) == "function", print = type(print) == "function",
      pcall = type(pcall) == "function", traceback = type(debug.traceback) == "function",
    },
  } }
end

return M
