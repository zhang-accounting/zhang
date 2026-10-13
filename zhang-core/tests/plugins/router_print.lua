-- A router plugin that prints while it handles a request, so a test sees where `print` goes: the host's
-- log, one line per call, prefixed with the script's name, with the arguments joined by tabs as Lua does.
local M = { name = "router-print", version = "0.1.0" }

function M.router(request)
  print("hello", 42, nil, true)
  print()
  return { status = 204 }
end

return M
