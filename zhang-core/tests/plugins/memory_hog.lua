-- A router plugin that allocates without bound, so a test sees the per-call memory budget stop it
-- with a clear error instead of the host running out of memory.
local M = { name = "memory-hog", version = "0.1.0" }

function M.router(request)
  local hoard = {}
  while true do
    hoard[#hoard + 1] = string.rep("x", 1024 * 1024, tostring(#hoard))
  end
end

return M
