-- A router plugin that fills its Lua memory, then raises and catches errors in a loop: `rounds` rounds of a plain error
-- and a memory error (trying to hold more small tables), then `errors` plain errors. Every error is thrown with an
-- exception object; were that object to cost memory that stays, or to grow the memory past the Lua memory's edge for
-- the next round to fill, the rounds would hold more than the first fill did, and the instance would end up growing
-- past its limit, a trap. Answers how many tables the first fill held and how many the rest gained after it. Nothing
-- outside a pcall allocates once the memory is full: the filler is made once and the counters are integers.
local M = { name = "error-loop", version = "0.1.0" }

function M.router(request)
  local rounds = math.tointeger(tonumber(request.query.rounds[1]))
  local errors = math.tointeger(tonumber(request.query.errors[1]))
  local strings, list, count = {}, nil, 0
  pcall(function() while true do strings[#strings + 1] = string.rep("x", 1024 * 1024) end end)
  local function filler() while true do list = { next = list }; count = count + 1 end end
  pcall(filler)
  local first = count
  for _ = 1, rounds do
    pcall(error, "caught", 0)
    pcall(filler)
  end
  for _ = 1, errors do pcall(error, "caught", 0) end
  pcall(filler)
  local gained = count - first
  strings, list = nil, nil
  collectgarbage()
  return { json = { first = first, gained = gained } }
end

return M
