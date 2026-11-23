-- A router plugin encoding a weak table of 15,000 entries with the Lua memory nearly exhausted, its free room swept
-- from 128 KiB to 8 MiB, so a test sees the encoder survive the emergency collection a refused growth runs: that
-- collection clears the entries of a weak table whose values are garbage and frees their key strings, which the
-- encoder must not read afterwards. Each run either raises Lua's memory error or writes entries that are whole:
-- well-formed keys, each with its value (never a dropped value written as null), and every entry when the values are
-- kept alive elsewhere; with 8 MiB of room, the kept entries must encode. Answers the runs, how many encoded, and the
-- failures.
local M = { name = "json-weak", version = "0.1.0" }

local ENTRIES = 15000
local PATTERN = "^weak%-key%-%d%d%d%d%d%d%-" .. string.rep("z", 48) .. "$"

-- How much Lua memory there is, as collectgarbage("count") counts it: what the count reaches once blocks of 1 MiB,
-- then 64 KiB, then 4000 bytes no longer fit. The Lua memory is the instance's memory limit less a reserve for the
-- host, and its edge is where the interpreter's memory stops growing, which takes in the module's stack and what the
-- allocator spends beside Lua's objects: a count reached the way the runs below fill it is the edge they meet
local function budget()
  collectgarbage()
  collectgarbage()
  local hog = {}
  for _, size in ipairs({ 1048576, 65536, 4000 }) do
    local base = string.rep("h", size)
    while pcall(function() hog[#hog + 1] = base .. #hog end) do end
  end
  local full = collectgarbage("count") * 1024
  hog = nil
  collectgarbage()
  collectgarbage()
  return full
end

function M.router(request)
  local BUDGET = budget()
  local runs, encoded, failures = 0, 0, {}
  for _, kept in ipairs({ false, true }) do
    for _, room in ipairs({ 128, 256, 512, 768, 1024, 1536, 2048, 3072, 4096, 6144, 8192 }) do
      collectgarbage()
      collectgarbage()
      local keep, weak = {}, setmetatable({}, { __mode = "v" })
      for i = 1, ENTRIES do
        local value = { i }
        keep[i] = value
        weak[string.format("weak-key-%06d-%s", i, string.rep("z", 48))] = value
      end
      -- fill the budget up to `room`, in large blocks first, one allocation per block and no collection meanwhile
      local hog = {}
      collectgarbage("stop")
      pcall(function()
        for _, size in ipairs({ 1048576, 65536, 4000 }) do
          local base = string.rep("h", size)
          while BUDGET - collectgarbage("count") * 1024 > room * 1024 + 8192 + 2 * size do hog[#hog + 1] = base .. #hog end
        end
      end)
      collectgarbage("restart")
      if not kept then keep = nil end
      local ok, text = pcall(json.encode, weak)
      hog = nil
      collectgarbage()
      runs = runs + 1
      local run = (kept and "kept " or "garbage ") .. room .. " KiB: "
      if not ok then
        if text ~= "not enough memory" or (kept and room == 8192) then failures[#failures + 1] = run .. tostring(text) end
      else
        encoded = encoded + 1
        local entries = 0
        for key, value in pairs(json.decode(text)) do
          entries = entries + 1
          if type(key) ~= "string" or not key:find(PATTERN) then failures[#failures + 1] = run .. "corrupt key " .. string.format("%q", tostring(key)):sub(1, 60) end
          if type(value) ~= "table" or value[1] == nil then failures[#failures + 1] = run .. "dropped value of " .. tostring(key):sub(1, 20) end
        end
        if kept and entries ~= ENTRIES then failures[#failures + 1] = run .. entries .. " entries of " .. ENTRIES end
      end
      if #failures > 10 then break end
    end
  end
  return { json = { runs = runs, encoded = encoded, failures = json.array(failures) } }
end

return M
