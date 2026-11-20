-- A router plugin answering what zhang.ledger_info gives, plus the config it was declared with.
local M = { name = "router-info", version = "0.1.0" }

function M.router(request)
  local info = zhang.ledger_info()
  info.greeting = zhang.config("greeting")
  info.missing = zhang.config("no-such-key")
  info.abi = zhang.config("zhang.abi")
  info.api_version = zhang.api_version
  return { json = info }
end

return M
