-- A script that does not parse, so a test sees the load error carry the Lua line: line 4 below.
local M = { name = "syntax-error", version = "0.1.0" }

function M.router(request) return { status = = 200 } end

return M
