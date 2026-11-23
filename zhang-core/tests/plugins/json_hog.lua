-- A router plugin whose JSON outgrows the Lua memory, so a test sees the codec's buffers and the values it builds
-- count against it: `op=encode` writes a document larger than what is left, `op=decode` reads one whose tables are,
-- and `op=caught` catches the decode's memory error and answers it. Sized for the 224 MiB of Lua memory of the default
-- limit (ZHANG_PLUGIN_MAX_MEMORY, 256 MiB).
local M = { name = "json-hog", version = "0.1.0" }

-- 5 million empty arrays: 15 MB of text, well over 224 MiB of tables
local function hog_text() return "[" .. string.rep("[],", 5000000) .. "[]]" end

function M.router(request)
  local op = request.query.op[1]
  if op == "encode" then
    -- 140 MB of strings, which fit, and a document of more than that, which does not fit beside them
    local rows = {}
    for i = 1, 140000 do rows[i] = string.rep("x", 1000) .. i end
    return { json = { size = #json.encode(rows) } }
  elseif op == "decode" then
    return { json = { size = #json.decode(hog_text()) } }
  end
  local ok, err = pcall(json.decode, hog_text())
  collectgarbage()
  return { json = { ok = ok, error = err, collector = collectgarbage("isrunning") } }
end

return M
