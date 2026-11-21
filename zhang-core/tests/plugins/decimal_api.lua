-- A router answering, for each path, what one use of zhang.decimal gives, or the error it raises as
-- {error = "..."}: the rules around the arithmetic the parity test covers.
local M = { name = "decimal-api", version = "0.1.0" }
local d = zhang.decimal

local cases = {
  -- values: a string, an integer, a decimal value, NULL
  ["/values"] = function()
    return {
      tostring(d("007.50")), tostring(d("+.5")), tostring(d("5.")), tostring(d("-0.00")), tostring(d(12)), tostring(d(-3)),
      tostring(d(math.mininteger)), d(d("1.5")) == d("1.50"), tostring(d.sum({})), tostring(d.sum({ "1.5", 2, d("-0.25") })),
      d.is_decimal(d("1")), d.is_decimal("1"), d.is_decimal({ type = "decimal", value = "1" }),
      getmetatable(d("1")), type(d), d("12.5").type, d("12.5").value,
    }
  end,
  ["/null"] = function() return { d(nil), d(json.null) } end,
  -- comparisons: by value; a string or an integer on either side of < and <=, never ==
  ["/compare"] = function()
    return {
      d("1") == d("1.00"), d("1") ~= d("1.01"), d("-0") == d("0.000"), d("2") < "10", "10" < d("2"), 3 <= d("3.0"),
      d("1") == "1", d.cmp("-1.5", "-1.50"), d.cmp("1", 2), d.sign("-0.0"), d.sign("-0.1"), d.is_zero("0.000"),
      d("9.99") < d("10"), d("-10") < d("-9.99"), d("0") == json.null, d("1") == { type = "decimal", value = "1" },
    }
  end,
  -- the text: tostring, concatenation, json.encode (a decimal value is its text) and a query parameter
  ["/text"] = function()
    local x = d("-1.50") + 1
    return {
      tostring(x), "x = " .. x, x .. "!", json.encode({ x = x, list = { d("0.10") + d("0.2") } }),
      zhang.query_params("SELECT date WHERE number > :min", { min = d("12.50") }),
    }
  end,
  -- what is refused, and where: the error names the script's line (a call in a tail position would name the caller's)
  ["/float"] = function()
    local x = d(0.1)
    return x
  end,
  ["/float-operand"] = function() return d("1") + 0.5 end,
  ["/exponent"] = function()
    local x = d("1e3")
    return x
  end,
  ["/text-refused"] = function()
    local refused = json.array()
    for _, text in ipairs({ "", ".", "-", "+-1", "1.2.3", " 1", "1 ", "1_000", "0x10", "١", "1,5", "--1", "Infinity", "nan" }) do
      local ok, message = pcall(d, text)
      refused[#refused + 1] = ok and ("accepted " .. text) or message
    end
    return refused
  end,
  ["/digits"] = function()
    local fits = d(string.rep("9", 500) .. "." .. string.rep("9", 500))
    return {
      #tostring(fits + fits), tostring(fits - fits), select(2, pcall(d, string.rep("1", 1001))),
      select(2, pcall(d, "0." .. string.rep("0", 1000))), select(2, pcall(d, string.rep("1", 2000))),
    }
  end,
  ["/multiply"] = function() return d("2") * d("3") end,
  ["/divide"] = function() return d("2") / 3 end,
  ["/null-arithmetic"] = function() return d(nil) + d("1") end,
  -- a value never changes: one whose text a script replaced is not written
  ["/changed"] = function()
    local x = d("1") + 0
    x.value = 5
    return json.encode(x)
  end,
  ["/sealed"] = function() return setmetatable(d("1"), {}) end,
  ["/table-operand"] = function() return d.add({}, "1") end,
}

function M.router(request)
  local case = cases[request.path]
  local ok, result = pcall(case)
  if not ok then return { json = { error = result } } end
  return { json = result }
end

return M
