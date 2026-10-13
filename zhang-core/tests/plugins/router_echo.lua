-- A router plugin that answers every request with the request itself: status 201, a custom `x-echo`
-- header and the request as a JSON body, so a test sees exactly what the plugin received. The Lua twin
-- of router.wat.
local M = { name = "router-echo", version = "0.1.0", types = { "Router" } }

function M.router(request)
  return { status = 201, headers = { ["x-echo"] = "lua" }, json = request }
end

return M
