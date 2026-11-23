-- A router plugin that holds `mib` MiB of Lua strings, 1 MiB each, and answers how many it holds, so a test sees how
-- much Lua memory a script gets under the memory limit of its instance (ZHANG_PLUGIN_MAX_MEMORY). With `tables`, it
-- holds empty tables until the memory runs out instead: small objects, whose allocator overhead Lua does not count.
-- With `caught`, it catches the memory error of holding strings without end, frees them, and fails: with Lua's message
-- in its own words (`caught=again`), or with an unrelated error (`caught=other`).
local M = { name = "memory-grow", version = "0.1.0" }

function M.router(request)
  local held = {}
  if request.query.tables then
    while true do held[#held + 1] = {} end
  end
  if request.query.caught then
    local _, err = pcall(function() while true do held[#held + 1] = string.rep("x", 1024 * 1024) end end)
    held = nil
    collectgarbage()
    if request.query.caught[1] == "again" then error("could not hold it all: " .. tostring(err)) end
    error("something else went wrong")
  end
  for i = 1, math.tointeger(tonumber(request.query.mib[1])) do held[i] = string.rep("x", 1024 * 1024) end
  return { json = { held = #held } }
end

return M
