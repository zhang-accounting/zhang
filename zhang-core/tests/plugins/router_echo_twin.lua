-- A second router plugin named like router_echo.lua, with another version and answer, so a test sees
-- that the first of two scripts sharing a name serves the route, and the later one is reported.
local M = { name = "router-echo", version = "0.2.0" }

function M.router(request)
  return { status = 200, body = "twin" }
end

return M
