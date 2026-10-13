-- A router plugin answering the query schema, as zhang.query_schema reads it from the host.
local M = { name = "router-schema", version = "0.1.0" }

function M.router(request)
  return { json = zhang.query_schema() }
end

return M
