-- A router plugin that checks the rules of the `json` table inside the interpreter module and answers the checks
-- that failed, so a test sees the C codec (tools/lua-runtime/json.cpp) keep them: encoding (numbers, escapes, UTF-8,
-- sorted keys, the object marker, cycles, nesting) and strict decoding (numbers, strings, positions in errors, 1000
-- levels of nesting both ways, which must not grow the wasm stack).
local M = { name = "json-rules", version = "0.1.0" }

function M.router(request)
  local failures, count = {}, 0
  local function show(v) if type(v) == "string" then return string.format("%q", v) end return tostring(v) end
  local function eq(name, got, want)
    count = count + 1
    if got ~= want then failures[#failures + 1] = name .. ": got " .. show(got) .. ", want " .. show(want) end
  end
  local function err(name, f, want)
    count = count + 1
    local ok, e = pcall(f)
    if ok then failures[#failures + 1] = name .. ": no error, want " .. show(want)
    elseif not tostring(e):find(want, 1, true) then failures[#failures + 1] = name .. ": error " .. show(e) .. ", want " .. show(want) end
  end
  local E, D = json.encode, json.decode

  -- encode scalars
  eq("nil", E(nil), "null"); eq("none", E(), "null"); eq("null", E(json.null), "null")
  eq("true", E(true), "true"); eq("false", E(false), "false")
  eq("0", E(0), "0"); eq("-1", E(-1), "-1")
  eq("maxint", E(math.maxinteger), "9223372036854775807"); eq("minint", E(math.mininteger), "-9223372036854775808")
  eq("1.5", E(1.5), "1.5"); eq("0.1", E(0.1), "0.1"); eq("third", E(1/3), "0.3333333333333333")
  eq("1.0", E(1.0), "1.0"); eq("-0.0", E(-0.0), "-0.0"); eq("1e20", E(1e20), "1e+20"); eq("1e15", E(1e15), "1e+15")
  eq("2^53", E(2.0^53), "9007199254740992.0"); eq("0.1+0.2", E(0.1 + 0.2), "0.30000000000000004")
  eq("1e-7", E(1e-7), "1e-07"); eq("1e14", E(1e14), "100000000000000.0"); eq("pi", E(math.pi), "3.141592653589793")
  eq("2^63f", E(2.0^63), "9.223372036854776e+18"); eq("tiny", E(5e-324), "4.94065645841247e-324")
  eq("nan", E(0/0), "null"); eq("inf", E(math.huge), "null"); eq("-inf", E(-math.huge), "null")
  -- strings
  eq("plain", E("abc"), '"abc"'); eq("empty", E(""), '""')
  eq("quote", E('a"b\\c/d'), '"a\\"b\\\\c/d"')
  eq("controls", E("\0\1\b\f\n\r\t\11\31\127"), '"\\u0000\\u0001\\b\\f\\n\\r\\t\\u000b\\u001f\\u007f"')
  eq("utf8", E("héllo 世界 \u{1F600}"), '"héllo 世界 \u{1F600}"')
  eq("u2028", E("a\u{2028}b\u{2029}"), '"a\\u2028b\\u2029"'); eq("bom", E("\u{FEFF}"), '"\\ufeff"')
  eq("c1", E("\u{85}\u{AD}"), '"\\u0085\\u00ad"'); eq("fff0", E("\u{FFF0}\u{FFFF}"), '"\\ufff0\\uffff"')
  eq("10000", E("\u{10000}\u{10FFFF}"), '"\u{10000}\u{10FFFF}"'); eq("200b", E("\u{200B}"), '"\u{200B}"')
  eq("600", E("\u{600}\u{604}\u{605}\u{70F}\u{17B4}\u{17B5}\u{206F}"), '"\\u0600\\u0604\u{605}\\u070f\\u17b4\\u17b5\\u206f"')
  err("bad utf8", function() return E("a\255") end, "not UTF-8 text: the byte at offset 1 is not UTF-8")
  err("truncated utf8", function() return E("\195") end, "offset 0")
  err("surrogate utf8", function() return E("\237\160\128") end, "offset 0")
  err("overlong", function() return E("\192\128") end, "offset 0")
  err("past 10ffff", function() return E("\244\144\128\128") end, "offset 0")
  err("bad key utf8", function() return E({["\255"] = 1}) end, "not UTF-8")
  -- tables
  eq("{}", E({}), "[]"); eq("object()", E(json.object()), "{}"); eq("array()", E(json.array()), "[]")
  eq("list", E({1, 2, 3}), "[1,2,3]"); eq("holes", E({1, nil, 3}), "[1,null,3]")
  eq("ten", E({[1] = 1, [10] = 2}), "[1,null,null,null,null,null,null,null,null,2]")
  eq("sparse", E({[1] = 1, [20] = 2}), '{"1":1,"20":2}'); eq("eleven", E({[11] = 1}), '{"11":1}')
  eq("half", E({[1] = 1, [2] = 2, [3] = 3, [4] = 4, [5] = 5, [6] = 6, [12] = 7}), "[1,2,3,4,5,6,null,null,null,null,null,7]")
  eq("sorted", E({b = 1, a = 2, c = {z = 1, y = 2}}), '{"a":2,"b":1,"c":{"y":2,"z":1}}')
  eq("mixed", E({ [3] = "c", [1.5] = "b", ["10"] = "d", [-1] = "a" }), '{"-1":"a","1.5":"b","3":"c","10":"d"}')
  eq("numfirst", E({ [2] = 1, [10] = 2, a = 3, ["1"] = 4, [0.5] = 5 }), '{"0.5":5,"2":1,"10":2,"1":4,"a":3}')
  eq("bytes", E({ ["b"] = 1, ["B"] = 2, ["é"] = 3, ["a\0b"] = 4, ["a"] = 5 }), '{"B":2,"a":5,"a\\u0000b":4,"b":1,"é":3}')
  eq("bigfloatkey", E({ [2.0^63] = 1, [math.huge] = 2, [-math.huge] = 3, [math.maxinteger] = 4, [-2.0^64] = 5, [math.mininteger] = 6 }),
     '{"-inf":3,"-1.8446744073709552e+19":5,"-9223372036854775808":6,"9223372036854775807":4,"9.223372036854776e+18":1,"inf":2}')
  eq("object marker wins", E(json.object({1, 2})), '{"1":1,"2":2}')
  eq("n is a key", E({n = 3}), '{"n":3}'); eq("pack", E(table.pack(1, 2)), '{"1":1,"2":2,"n":2}')
  eq("array marker", E(json.array({a = 1})), '{"a":1}')
  eq("jsontype", E(setmetatable({}, {__jsontype = "object"})), "{}")
  eq("nested", E({a = {b = {c = {}}}}), '{"a":{"b":{"c":[]}}}')
  eq("null in", E({json.null, a = json.null}), '{"1":null,"a":null}')
  local shared = {1}; eq("dag", E({shared, shared}), "[[1],[1]]")
  local cyc = {}; cyc.self = cyc; err("cycle", function() return E(cyc) end, "reference cycle")
  local cyc2 = {{}}; cyc2[1][1] = cyc2; err("cycle2", function() return E(cyc2) end, "reference cycle")
  err("function", function() return E({f = print}) end, "type 'function' is not supported by JSON.")
  err("bool key", function() return E({[true] = 1}) end, "type 'boolean' is not supported as a key by JSON.")
  err("thread", function() return E(coroutine.create(print)) end, "type 'thread' is not supported by JSON.")
  local proxy = setmetatable({}, {__index = {a = 1}, __pairs = function() error("no") end}); eq("raw", E(proxy), "[]")
  local longkey = string.rep("k", 100); eq("longkey", E({[longkey] = 1, a = 2}), '{"a":2,"' .. longkey .. '":1}')
  local deep = {}; local cur = deep; for i = 1, 999 do cur[1] = {}; cur = cur[1] end
  eq("depth 1000", #E(deep), 2000)
  cur[1] = {}; err("depth 1001", function() return E(deep) end, "nested deeper than 1000 levels")
  -- a number key and the same key as a string would be written twice
  err("dup int", function() return E({[1] = "a", ["1"] = "b"}) end, 'both the number key 1 and the string key "1": JSON would write the key twice')
  err("dup float", function() return E({[1.5] = 1, ["1.5"] = 2, z = 3}) end, 'number key 1.5 and the string key "1.5"')
  err("dup inf", function() return E({[math.huge] = 1, inf = 2}) end, 'number key inf and the string key "inf"')
  eq("no dup", E({[1] = 1, ["01"] = 2, ["1.0"] = 3, [2] = 4}), '{"1":1,"2":4,"01":2,"1.0":3}')
  -- the markers: sealed, a look-alike for getmetatable, swappable by json.object/json.array only
  eq("marker kind object", getmetatable(json.object()).__jsontype, "object"); eq("marker kind array", getmetatable(json.array()).__jsontype, "array")
  local lookalike = getmetatable(json.object())
  err("lookalike read-only", function() lookalike.__jsontype = "array" end, "the json markers are read-only")
  err("lookalike no mode", function() lookalike.__mode = "v" end, "the json markers are read-only")
  err("lookalike sealed", function() setmetatable(lookalike, nil) end, "cannot change a protected metatable")
  eq("lookalike kind kept", getmetatable(D("{}")).__jsontype, "object")
  err("decoded sealed", function() setmetatable(D("{}"), {}) end, "cannot change a protected metatable")
  err("array() sealed", function() setmetatable(json.array(), {}) end, "cannot change a protected metatable")
  local copied = setmetatable({}, {__index = {fallback = 1}}); for k, v in pairs(D('{"a":2}')) do copied[k] = v end
  eq("copy first", copied.a + copied.fallback, 3)
  -- rawset can shadow a look-alike's fields, but not the markers themselves: decoded data stays strong
  rawset(lookalike, "__mode", "v")
  local held = D('{"a":{"b":1}}'); collectgarbage(); collectgarbage()
  eq("sealed marker stays strong", E(held), '{"a":{"b":1}}')
  local shadow, alive = {}, {}; for i = 1, 50 do alive[i] = {i}; shadow["s" .. i] = alive[i] end
  eq("weak lookalike written whole", #E(setmetatable(shadow, lookalike)) > 300, true)
  rawset(lookalike, "__mode", nil)
  eq("reobject", E(json.object(D("[]"))), "{}"); eq("rearray", E(json.array(D("{}"))), "[]"); eq("object(false)", E(json.object(false)), "{}")
  err("protected", function() return json.object(setmetatable({}, {__metatable = "locked"})) end, "cannot change a protected metatable")
  err("object(1)", function() return json.object(1) end, "table expected, got number")
  eq("lookalike marks", E(setmetatable({}, getmetatable(json.object()))), "{}")
  -- a weak table is written from a strong copy: every entry it holds, each key with its value
  local kept, weak = {}, setmetatable({}, {__mode = "v"})
  for i = 1, 300 do kept[i] = {i}; weak["w" .. i] = kept[i] end
  local back = D(E(weak)); local entries = 0; for _ in pairs(back) do entries = entries + 1 end
  eq("weak entries", entries, 300); eq("weak value", back.w123[1], 123)
  local weakarray = setmetatable({}, {__mode = "k"}); for i = 1, 5 do weakarray[i] = i end
  eq("weak array", E(weakarray), "[1,2,3,4,5]")
  local wide = {}; for i = 1, 5000 do wide["k" .. i] = i end
  local text = E(wide); eq("wide first", text:sub(1, 8), '{"k1":1,'); eq("wide back", D(text).k4321, 4321)

  -- decode scalars
  eq("d null", D("null"), json.null); eq("d true", D("true"), true); eq("d false", D(" false "), false)
  eq("d 0", math.type(D("0")), "integer"); eq("d -0", math.type(D("-0")), "integer")
  eq("d -0.0", 1 / D("-0.0"), -math.huge); eq("d 1.5", D("1.5"), 1.5); eq("d 1e2", math.type(D("1e2")), "float")
  eq("d 1E+2", D("1E+2"), 100.0); eq("d 1e-2", D("1e-2"), 0.01)
  eq("d max", D("9223372036854775807"), math.maxinteger); eq("d min", D("-9223372036854775808"), math.mininteger)
  eq("d 2^63", D("9223372036854775808"), 2.0^63); eq("d 2^63 type", math.type(D("9223372036854775808")), "float")
  eq("d -2^63-1", D("-9223372036854775809"), -2.0^63); eq("d 20 digits", D("12345678901234567890"), 12345678901234567890.0)
  eq("d 1.0", math.type(D("1.0")), "float")
  err("d 1e400", function() return D("1e400") end, "number out of range at line 1, column 1")
  err("d -1e400", function() return D("[-1e400]") end, "number out of range at line 1, column 2")
  eq("d 1e-400", D("1e-400"), 0.0)
  eq("d long frac", D("0.1000000000000000055511151231257827021181583404541015625"), 0.1)
  eq("d many digits", D("123456789012345678901234567890.5e-10"), tonumber("123456789012345678901234567890.5e-10"))
  for _, s in ipairs({"0.30000000000000004", "2.2250738585072014e-308", "4.9406564584124654e-324", "1.7976931348623157e308",
    "9007199254740993", "9007199254740993.0", "123.456e5", "0.000001", "1e22", "1e23", "8.98846567431158e307"}) do
    eq("d " .. s, D(s), tonumber(s))
  end
  err("d 01", function() return D("01") end, "invalid number (leading zero) at line 1, column 1")
  err("d 1.", function() return D("1.") end, "invalid number at line 1, column 1")
  err("d .5", function() return D(".5") end, "no valid JSON value at line 1, column 1")
  err("d -", function() return D("-") end, "invalid number at line 1, column 1")
  err("d +1", function() return D("+1") end, "no valid JSON value at line 1, column 1")
  err("d 1e", function() return D("1e") end, "invalid number")
  err("d 1e+", function() return D("1e+") end, "invalid number")
  err("d hex", function() return D("0x10") end, "unexpected data at byte 2")
  err("d NaN", function() return D("NaN") end, "no valid JSON value at line 1, column 1")
  err("d Infinity", function() return D("[Infinity]") end, "no valid JSON value at line 1, column 2")
  err("d -Infinity", function() return D("-Infinity") end, "invalid number")
  err("d trueX", function() return D("truex") end, "no valid JSON value")
  err("d truncated literal", function() return D("[t]") end, "no valid JSON value at line 1, column 2")
  -- strings
  eq("d empty str", D('""'), ""); eq("d escapes", D('"\\"\\\\\\/\\b\\f\\n\\r\\t"'), '"\\/\b\f\n\r\t')
  eq("d \\u0041", D('"\\u0041\\u00e9\\u4E16"'), "Aé世"); eq("d pair", D('"\\ud83d\\ude00"'), "\u{1F600}")
  eq("d nul", D('"a\\u0000b"'), "a\0b"); eq("d del", D('"\127"'), "\127"); eq("d utf8", D('"héllo 世界 \u{1F600}"'), "héllo 世界 \u{1F600}")
  err("d lone high", function() return D('"\\ud800"') end, "unpaired surrogate in \\u escape at line 1, column 2")
  err("d lone low", function() return D('"\\udc00"') end, "unpaired surrogate")
  err("d high+bmp", function() return D('"\\ud800\\u0041"') end, "unpaired surrogate")
  err("d short u", function() return D('"\\u12"') end, "invalid \\u escape at line 1, column 2")
  err("d bad esc", function() return D('"\\x"') end, "invalid escape at line 1, column 2")
  err("d raw ctl", function() return D('"a\nb"') end, "control character in string at line 1, column 3")
  err("d raw nul", function() return D('"a\0b"') end, "control character in string")
  err("d bad utf8", function() return D('"a\255"') end, "invalid UTF-8 at line 1, column 3")
  err("d truncated utf8", function() return D('"\195"') end, "invalid UTF-8")
  err("d utf8 at end", function() return D('"\226\130') end, "invalid UTF-8")
  err("d surrogate utf8", function() return D('"\237\160\128"') end, "invalid UTF-8")
  err("d unterminated", function() return D('"abc') end, "unterminated string at line 1, column 1")
  err("d unterminated esc", function() return D('"abc\\') end, "unterminated string at line 1, column 1")
  err("d utf8 outside", function() return D("\255") end, "no valid JSON value at line 1, column 1")
  -- containers
  local a, o = D("[]"), D("{}")
  eq("d [] meta", getmetatable(a), getmetatable(json.array())); eq("d {} meta", getmetatable(o), getmetatable(json.object()))
  eq("d [] back", E(a), "[]"); eq("d {} back", E(o), "{}")
  eq("d nested", E(D(' { "b" : [1, {"c": null}, "x"], "a": {} } ')), '{"a":{},"b":[1,{"c":null},"x"]}')
  eq("d n obj", E(D('{"n": 5}')), '{"n":5}'); eq("d int keys stay strings", E(D('{"2":1,"10":2}')), '{"10":2,"2":1}')
  eq("d dup", D('{"a":1,"a":2}').a, 2); eq("d count", #D("[1,2,3,null,5]"), 5); eq("d null elem", D("[1,null]")[2], json.null)
  err("d [1,]", function() return D("[1,]") end, "no valid JSON value at line 1, column 4")
  err("d [1 2]", function() return D("[1 2]") end, "expected ',' or ']' at line 1, column 4")
  err("d {a:1,}", function() return D('{"a":1,}') end, "invalid key at line 1, column 8")
  err("d {a 1}", function() return D('{"a" 1}') end, "missing colon at line 1, column 6")
  err("d {a:}", function() return D('{"a": }') end, "invalid JSON: no valid JSON value at line 1, column 7")
  err("d {1:2}", function() return D("{1:2}") end, "invalid key at line 1, column 2")
  err("d {a:1 b}", function() return D('{"a":1 "b":2}') end, "expected ',' or '}' at line 1, column 8")
  err("d [", function() return D("[") end, "unterminated array at line 1, column 1")
  err("d {", function() return D("  {") end, "unterminated object at line 1, column 3")
  err("d [1", function() return D("[1") end, "unterminated array at line 1, column 1")
  err("d [[1]", function() return D("[[1]") end, "unterminated array at line 1, column 1")
  err("d {a:1", function() return D('{"a":1') end, "unterminated object at line 1, column 1")
  err("d {a", function() return D('{"a"') end, "unterminated object at line 1, column 1")
  err("d {a:", function() return D('{"a":') end, "unterminated object at line 1, column 1")
  err("d empty", function() return D("") end, "invalid JSON: no valid JSON value (reached the end)")
  err("d spaces", function() return D(" \n\t") end, "no valid JSON value (reached the end)")
  err("d trailing", function() return D("{} x") end, "invalid JSON: unexpected data at byte 4")
  eq("d trailing ws", E(D("{}\n\r\t ")), "{}")
  err("d bom", function() return D("\239\187\191{}") end, "no valid JSON value at line 1, column 1")
  err("d comment", function() return D("[1 /* x */]") end, "expected ',' or ']'")
  err("d lines", function() return D("[\n1,\n  x]") end, "no valid JSON value at line 3, column 3")
  err("d crlf", function() return D("[\r\n1,\r\n  x]") end, "no valid JSON value at line 3, column 3")
  err("d not string", function() return D(1) end, "json.decode expects a string, got number")
  err("d nil", function() return D() end, "json.decode expects a string, got nil")
  err("d embedded nul", function() return D("[1,\0]") end, "no valid JSON value at line 1, column 4")
  local deep_text = string.rep("[", 1000) .. string.rep("]", 1000)
  eq("d depth 1000", E(D(deep_text)), deep_text)
  err("d depth 1001", function() return D(string.rep("[", 1001) .. string.rep("]", 1001)) end, "nested deeper than 1000 levels at line 1, column 1001")
  err("d depth 1001 obj", function() return D(string.rep('{"a":', 1001)) end, "nested deeper than 1000 levels")
  err("d huge depth", function() return D(string.rep("[", 1000000)) end, "nested deeper than 1000 levels")
  -- round trips
  local doc = '{"a":[1,2.5,"x",true,false,null,{"b":{}}],"c":"\\u2028\\u0000é","d":-9223372036854775808}'
  eq("round trip", E(D(doc)), doc)
  -- error positions include the caller when not called through pcall directly
  local ok, e = pcall(function() return D("[") end)
  eq("where", e:find("json_rules.lua:%d+: invalid JSON") ~= nil, true)
  -- the codec pauses the collector while it works; an error must not leave it paused
  eq("collector", collectgarbage("isrunning"), true)

  return { json = { checks = count, failures = json.array(failures) } }
end

return M
