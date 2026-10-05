return {
  process = function(directives)
    zhang.emit_error("Lua processor loaded directly from business.script")
    return directives
  end,
  router = function(request)
    local result = zhang.query("SELECT account, sum(number) AS total WHERE account ~ '^Expenses' GROUP BY account ORDER BY account")
    return {status = 200, headers = {["content-type"] = "application/json"}, body = json.encode(result)}
  end,
}
