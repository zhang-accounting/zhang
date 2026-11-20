-- A script declaring a type Lua scripts cannot run as this round, which the interpreter refuses.
local M = { name = "declares-processor", version = "0.1.0", types = { "Router", "Processor" } }

function M.router(request)
  return { status = 200 }
end

return M
