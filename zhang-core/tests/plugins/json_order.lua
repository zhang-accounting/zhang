-- A router plugin encoding a table with many string keys, so a test sees that two requests give the same
-- bytes: the interpreter's string hash seed is fixed, so `pairs` visits the keys in the same order every time.
local M = { name = "json-order", version = "0.1.0" }

function M.router(request)
  local record = {}
  for _, key in ipairs({ "payee", "narration", "date", "account", "amount", "currency", "tags", "links", "flag", "meta", "id", "file" }) do
    record[key] = request.path .. key
  end
  record.nested = { z = 1, y = 2, x = 3, w = 4, v = 5, u = 6, t = 7 }
  return { json = record }
end

return M
