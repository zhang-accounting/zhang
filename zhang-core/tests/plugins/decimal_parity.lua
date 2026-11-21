-- A router computing with zhang.decimal what the request body asks, so that a test compares it with the host's own
-- decimals on many inputs at once. The body is {"cases": [[a, b], ...]}, decimals as strings; the answer gives, for
-- each case, the list
--
--   text of a, a + b, a - b, -a, |a|, cmp(a, b), a == b, a < b, a <= b, a is zero, the sign of a,
--   add(a, b) and sub(a, b) of the strings, a + b with the string b, the string a - b, json.encode({a + b}),
--
-- and {"sums": [...]} the sum of each list of {"lists": [[...], ...]}.
local M = { name = "decimal-parity", version = "0.1.0", types = { "Router" } }
local d = zhang.decimal

function M.router(request)
  local body = json.decode(request.body)
  local results = json.array()
  for index, case in ipairs(body.cases or {}) do
    local a, b = d(case[1]), d(case[2])
    local sum, difference = a + b, a - b
    results[index] = {
      tostring(a), tostring(sum), tostring(difference), tostring(-a), tostring(a:abs()),
      a:cmp(b), a == b, a < b, a <= b, a:is_zero(), a:sign(),
      tostring(d.add(case[1], case[2])), tostring(d.sub(case[1], case[2])),
      tostring(a + case[2]), tostring(case[1] - b), json.encode({ sum }),
    }
  end
  local sums = json.array()
  for index, list in ipairs(body.lists or {}) do sums[index] = d.sum(list) end
  return { json = { results = results, sums = sums } }
end

return M
