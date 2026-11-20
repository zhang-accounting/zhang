-- A router plugin whose answer is not a table, which the host refuses as an invalid response.
local M = { name = "router-malformed", version = "0.1.0" }

function M.router(request)
  return "not a response"
end

return M
