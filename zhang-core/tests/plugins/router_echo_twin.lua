-- A second router plugin named like router_echo.lua, with another version and answer, so a test sees
-- which of two scripts sharing a name serves the route (the first one declared).
local M = { name = "router-echo", version = "0.2.0" }

function M.router(request)
  return { status = 200, body = "twin" }
end

return M
