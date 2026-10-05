return {run = function(values)
  local total = 0
  for _, value in ipairs(values) do total = total + value end
  zhang.emit_error("Changed Lua script ran")
  return {sum = total * 10, now = zhang.now()}
end}
