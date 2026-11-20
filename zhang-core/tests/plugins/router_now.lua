-- A router plugin answering what zhang.now gives for the request: the time, date and timezone.
local M = { name = "router-now", version = "0.1.0" }

function M.router(request)
  return { json = zhang.now() }
end

return M
