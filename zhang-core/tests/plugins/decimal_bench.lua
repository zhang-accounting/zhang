-- A router for the cost test of zhang.decimal (a script has no clock, so the test times the calls): `/<work>?n=<n>`
-- does one kind of work n times, and the test divides the difference with n = 0 by n. The work: adding amounts
-- (values, strings, a running balance as a budget keeps one), adding long decimals, reading decimals, comparing, a
-- sum of a list; and, for what a batched host operation would cost instead, a crossing to the host and back
-- (zhang.ledger_info, the cheapest host function) and the JSON of a list of amounts both ways.
local M = { name = "decimal-bench", version = "0.1.0" }
local d = zhang.decimal

local work = {
  noop = function(n)
    local x = 0
    for i = 1, n do x = x + i end
    return x
  end,
  add = function(n)
    local a, b, x = d("1234.56"), d("-0.07"), nil
    for _ = 1, n do x = a + b end
    return tostring(x)
  end,
  add_string = function(n)
    local total = d("0")
    for _ = 1, n do total = total + "12.34" end
    return tostring(total)
  end,
  running = function(n)
    local available = d("0")
    for _ = 1, n do
      local assigned = available + "500.00"
      available = assigned - "123.45"
    end
    return tostring(available)
  end,
  long = function(n)
    local a, b, x = d(string.rep("7", 40) .. ".25"), d("-" .. string.rep("3", 30) .. ".125"), nil
    for _ = 1, n do x = a + b end
    return tostring(x)
  end,
  parse = function(n)
    local x
    for i = 1, n do x = d(i .. ".50") end
    return x and tostring(x) or ""
  end,
  compare = function(n)
    local a, b, count = d("12.30"), d("12.3"), 0
    for _ = 1, n do
      if a <= b then count = count + 1 end
    end
    return count
  end,
  sum = function(n)
    local list = {}
    for i = 1, n do list[i] = "12.34" end
    return tostring(d.sum(list))
  end,
  host = function(n)
    for _ = 1, n do zhang.ledger_info() end
    return n
  end,
  json = function(n)
    local list = {}
    for i = 1, n do list[i] = "12.34" end
    return #json.decode(json.encode(list))
  end,
}

function M.router(request)
  local n = math.tointeger(tonumber(request.query.n[1]))
  return { json = work[request.path:sub(2)](n) }
end

return M
