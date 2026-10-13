-- A router plugin that raises a Lua error while handling a request, so a test sees the host report
-- the failure with the script's line: `error` is called on line 6.
local M = { name = "router-trap", version = "0.1.0" }

function M.router(request)
  error("boom")
end

return M
