-- A router plugin that round-trips values through the json library and the host, so a test sees the
-- JSON rules hold: json.null decodes from and encodes to null, json.object() encodes as {} where an
-- empty table is [], an integer up to 2^63 stays exact, a NUL byte inside a string survives, a float
-- encodes with 14 significant digits, and a decimal the host sends as a string comes back untouched.
local M = { name = "json-roundtrip", version = "0.1.0" }

function M.router(request)
  local posted = json.decode(request.body)
  local rows = zhang.query("SELECT 1")
  return { json = {
    null = json.null,
    posted_null_is_null = posted.null == json.null,
    posted = posted,
    empty_object = json.object(),
    empty_table = {},
    empty_array = json.array(),
    filled_object = json.object({ k = "v" }),
    big = 4611686018427387904,
    big_back = posted.big,
    big_is_exact = posted.big == 4611686018427387904 and math.type(posted.big) == "integer",
    max = math.maxinteger,
    min = math.mininteger,
    float = 1.5,
    tenth = 0.1,
    third = 1 / 3,
    nul = "a\0b",
    nul_back = posted.nul,
    nul_back_len = #posted.nul,
    decimal = rows.dec,
    decimal_is_string = type(rows.dec) == "string",
    unicode = "héllo 世界 \u{1F600}",
    escaped = "quote\" backslash\\ newline\n tab\t",
    yes = true,
    no = false,
    absent = nil,
    bad_json = select(2, pcall(json.decode, "{\"a\": }")),
    trailing = select(2, pcall(json.decode, "{} x")),
  } }
end

return M
