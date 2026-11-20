-- A router plugin whose JSON outgrows the memory budget, so a test sees the codec's buffers and the values it builds
-- count against the budget: `op=encode` writes a document larger than what is left, `op=decode` reads one whose
-- tables are, and `op=caught` catches the decode's memory error and answers it.
local M = { name = "json-hog", version = "0.1.0" }

-- 1.5 million empty arrays: 4.5 MB of text, well over 64 MiB of tables
local function hog_text() return "[" .. string.rep("[],", 1500000) .. "[]]" end

function M.router(request)
  local op = request.query.op[1]
  if op == "encode" then
    local rows = {}
    for i = 1, 200000 do rows[i] = string.rep("x", 100) .. i end
    return { json = { size = #json.encode(rows) } }
  elseif op == "decode" then
    return { json = { size = #json.decode(hog_text()) } }
  end
  local ok, err = pcall(json.decode, hog_text())
  collectgarbage()
  return { json = { ok = ok, error = err, collector = collectgarbage("isrunning") } }
end

return M
