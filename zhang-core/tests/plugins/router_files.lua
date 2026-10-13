-- A router plugin that reads a file and lists a directory through zhang.read_file and zhang.list_dir
-- while it handles a request, and answers what each call gave or raised, so a test sees that a router
-- gets no file access even when its directive grants `allowed_paths`. The path comes from the `read`
-- config key. The Lua twin of router_files.wat.
local M = { name = "router-files", version = "0.1.0" }

local function attempt(call, path)
  local ok, result = pcall(call, path)
  if ok then return { ok = result } end
  return { error = result }
end

function M.router(request)
  local path = zhang.config("read")
  return { json = { file = attempt(zhang.read_file, path), dir = attempt(zhang.list_dir, ".") } }
end

return M
