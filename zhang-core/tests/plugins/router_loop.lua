-- A router plugin that never answers, so a test sees the host stop it at its timeout.
local M = { name = "router-loop", version = "0.1.0" }

function M.router(request)
  while true do end
end

return M
