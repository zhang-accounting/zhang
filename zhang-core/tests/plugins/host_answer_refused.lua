-- A router plugin that fills its memory budget, then asks the host `rounds` times for an answer too big for what is
-- left: its own source, through zhang.config("zhang.script"), which the test pads to almost 1 MiB. Lua refuses every
-- copy of the answer with a memory error the script catches; the host's block holding the answer must be freed all
-- the same, or the refused answers pile up in the host's memory, outside the budget, until the call fails.
local M = { name = "host-answer-refused", version = "0.1.0" }

function M.router(request)
  local rounds = math.tointeger(tonumber(request.query.rounds[1]))
  local hold = {}
  local function grow() hold[#hold + 1] = string.rep("x", 65536) end
  -- 64 KiB strings until one more does not fit
  while pcall(grow) do end
  local refused = 0
  for _ = 1, rounds do
    local ok, err = pcall(zhang.config, "zhang.script")
    if ok then error("the source fit in what is left of the budget") end
    if err ~= "not enough memory" then error("not the memory error: " .. tostring(err)) end
    refused = refused + 1
  end
  hold = nil
  collectgarbage()
  return { json = { refused = refused } }
end

return M
