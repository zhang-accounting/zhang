-- A router plugin that reads its config at the top level of the script, as an idiomatic script does, so a
-- test sees that registration (which runs the chunk) already has the directive's metas and the ledger's
-- options; the router answers what the top level read.
local M = { name = "config-top-level", version = "0.1.0" }

local greeting = zhang.config("greeting"):upper()
local title = zhang.config("title")

function M.router(request)
  return { json = { greeting = greeting, title = title } }
end

return M
