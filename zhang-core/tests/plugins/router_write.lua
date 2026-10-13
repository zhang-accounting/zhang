-- A router plugin that drives zhang.write.preview and zhang.write.commit with the request's own body, so a test
-- (or curl) sends any batch over HTTP and reads the host's answer back; the Lua twin of router_write.wat. The
-- second character of the request path selects the case; the body is the host's answer as a table, or the error
-- the call raised as `{error = "..."}`:
--
-- - `/preview`: zhang.write.preview of the body's `directives`
-- - `/commit`: zhang.write.commit of the body's `directives` with its `token`
-- - `/f`: a batch holding a float amount, which the lint refuses before the host sees it
-- - `/i`: a batch holding an integer where a string belongs, which the host refuses
-- - `/e`: an empty batch
-- - anything else: a commit with a token that is not a string
--
-- It declares the `writes` config of its directive (a JSON list), so one fixture covers every declaration.
local declared = zhang.config("writes")
local M = { name = "router-write", version = "0.1.0", writes = declared and json.decode(declared) or nil }

local cases = {
  p = function(request) return zhang.write.preview(json.decode(request.body).directives) end,
  c = function(request)
    local input = json.decode(request.body)
    return zhang.write.commit(input.directives, input.token)
  end,
  f = function()
    return zhang.write.preview({ { kind = "transaction", datetime = "2026-10-13", payee = "x", postings = {
      { account = "Assets:Cash", unit = { number = 12.3, commodity = "USD" } },
    } } })
  end,
  i = function() return zhang.write.preview({ { kind = "open", date = "2026-10-13", account = "Assets:Bank", metas = { { key = "n", value = 3 } } } }) end,
  e = function() return zhang.write.preview({}) end,
}

function M.router(request)
  local case = cases[request.path:sub(2, 2)] or function() return zhang.write.commit({ { kind = "open" } }, 42) end
  local ok, result = pcall(case, request)
  if not ok then return { json = { error = result } } end
  return { json = result }
end

return M
