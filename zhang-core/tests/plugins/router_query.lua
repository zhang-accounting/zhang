-- A router plugin that reads the ledger through the host: the body is a JSON array of what
-- zhang.query and zhang.ledger_info answered, and a query that fails raises an error the plugin
-- reports as a value. The Lua twin of router_query.wat.
local M = { name = "router-query", version = "0.1.0" }

function M.router(request)
  local rows = zhang.query("SELECT account, sum(position) AS balance GROUP BY account ORDER BY account")
  local info = zhang.ledger_info()
  local ok, failure = pcall(zhang.query, "FAIL")
  zhang.emit_error("only logged while routing")
  return { json = { rows, info, { ok = ok, message = failure } } }
end

return M
