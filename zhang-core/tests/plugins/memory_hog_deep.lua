-- A router plugin that reaches the memory budget while its Lua stack is deep: each level of the recursion
-- keeps a 64 KiB string alive. Unwinding from the memory error shrinks the Lua stack, and a test sees that
-- the budget is still what the host reports, not a plain Lua memory error.
local M = { name = "memory-hog-deep", version = "0.1.0" }

local keep = {}

local function dive(n)
  keep[#keep + 1] = string.rep("z", 64 * 1024, tostring(n))
  local a, b, c, d, e, f, g, h = n, n, n, n, n, n, n, n
  return dive(n + 1) + a + b + c + d + e + f + g + h
end

function M.router(request)
  return { json = dive(1) }
end

return M
