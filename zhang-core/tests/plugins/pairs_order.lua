-- A router plugin answering the order `pairs` visits 64 string keys in, so a test sees the string hash seed is
-- not the same on every request (a fixed seed lets keys be crafted to collide).
local M = { name = "pairs-order", version = "0.1.0" }

function M.router(request)
  local keys = {}
  for i = 1, 64 do keys["key" .. i] = true end
  local order = {}
  for key in pairs(keys) do order[#order + 1] = key end
  return { json = order }
end

return M
