-- A router plugin that drives zhang.query_params and zhang.query_schema with a fixed input per
-- request path, so one fixture covers every case; the body is the host's answer, or the error the
-- call raised as `{error = "..."}`. The Lua twin of router_query_params.wat:
--
-- - `/a`: a date, a str and a decimal parameter, made with the constructors
-- - `/b`: a parameter bound to nil, which is NULL
-- - `/c`: a parameter of an unknown type, as a raw {type, value} table
-- - `/d`: a parameter whose value is not of the shape its type takes
-- - `/e`: a parameter the query uses but the input does not bind
-- - `/f`: a parameter that is not a constructor's table
-- - `/g`: a decimal made from a Lua float, which the constructor refuses
-- - `/h`: a decimal made from a Lua integer, which passes as it is
-- - `/n`: no parameters at all, which must still cross as an empty object
-- - anything else: zhang.query_schema
local M = { name = "router-query-params", version = "0.1.0" }

local cases = {
  a = function()
    return zhang.query_params("SELECT date, narration WHERE date >= :from AND narration = :who AND number > :min", {
      from = zhang.date("2024-01-01"), who = zhang.str("lunch"), min = zhang.decimal("5.5"),
    })
  end,
  b = function() return zhang.query_params("SELECT date WHERE narration = :who", { who = zhang.str(nil) }) end,
  c = function() return zhang.query_params("SELECT date WHERE narration = :who", { who = { type = "money", value = "x" } }) end,
  d = function() return zhang.query_params("SELECT date WHERE date >= :from", { from = zhang.date(20240101) }) end,
  e = function() return zhang.query_params("SELECT date WHERE date >= :from", {}) end,
  f = function() return zhang.query_params("SELECT date WHERE date >= :from", { from = "2024-01-01" }) end,
  g = function() return zhang.query_params("SELECT date WHERE number > :min", { min = zhang.decimal(12.5) }) end,
  h = function() return zhang.query_params("SELECT date WHERE number > :min", { min = zhang.decimal(12) }) end,
  n = function() return zhang.query_params("SELECT date") end,
}

function M.router(request)
  local case = cases[request.path:sub(2, 2)] or zhang.query_schema
  local ok, result = pcall(case)
  if not ok then return { json = { error = result } } end
  return { json = result }
end

return M
