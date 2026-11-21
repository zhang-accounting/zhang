---
title: Lua Plugins
description: Write a router or an analyzer plugin as a Lua script, with nothing to compile. The module table, the zhang and json API, exact decimals, analyzers, the JSON rules, the sandbox, the limits and the errors.
sidebar:
  order: 9
---

A plugin can be a Lua script instead of a WebAssembly module. Copy a `.lua` file into your ledger and declare it like any plugin; Zhang runs it in a Lua 5.4 interpreter built into Zhang, in the same sandbox as a wasm plugin, with the same capabilities and the same `/api/plugins/{name}` route. There is nothing to compile and nothing to install.

Lua scripts are **router plugins** in this release: they serve pages and data, and read the ledger with queries. They can also be **analyzers**, experimentally: they read the ledger once it is loaded and publish diagnostics and tables ([Analyzers](#analyzers)). They compute with amounts exactly through [`zhang.decimal`](#exact-decimals). [Why no processors or mappers](#no-processors-or-mappers-yet) is explained below.

## A plugin in twenty lines

```zhang
option "features.plugin" "true"

plugin "plugins/food.lua"
```

```lua
-- plugins/food.lua: this month's food balances, as JSON
local M = { name = "food", version = "0.1.0", types = { "Router" } }

function M.router(request)
  local month = zhang.now().today:sub(1, 7) .. "-01"        -- "2026-10-01"
  local result = zhang.query_params(
    "SELECT account, sum(position) AS balance WHERE account ~ :re AND date >= :from GROUP BY account ORDER BY account",
    { re = zhang.str("^Expenses:Food"), from = zhang.date(month) }
  )
  if request.query.format and request.query.format[1] == "text" then
    local lines = {}
    for _, row in ipairs(result.rows) do lines[#lines + 1] = row[1] end
    return { body = table.concat(lines, "\n") }
  end
  return { status = 200, json = { since = month, columns = result.columns, rows = result.rows } }
end

return M
```

`GET /api/plugins/food/` answers the JSON, `GET /api/plugins/food/?format=text` the account names. Editing `plugins/food.lua` reloads the ledger under `zhang serve`, as editing a wasm module does.

## The module table

The script is one chunk that returns a table:

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | The plugin's name, as `GET /api/plugins` lists it and as the route `/api/plugins/{name}` names it. When two router plugins share a name, the first one declared serves the route, and the later one is reported as a [`PluginError`](/reference/error-codes/#pluginerror) on its directive. |
| `version` | yes | The plugin's version, a string. |
| `types` | no | The plugin types, a list: `{ "Router" }`, the default, `{ "Analyzer" }`, or both. A list with no type or another type, or a map where a list was meant, does not load. |
| `router` | for a router | The function that answers a request. |
| `analyze` | for an analyzer | The function that analyses the ledger ([Analyzers](#analyzers)). |
| `analysis_abi`, `namespace`, `consumes`, `open_meta`, `diagnostics`, `tables`, `columns` | for an analyzer | Its manifest, the fields of [the manifest](/developers/writing-plugins/#the-manifest) as Lua tables: `analysis_abi = 1`, `consumes = { "custom:shop" }`, `tables = { { name = "items", kind = "records", columns = { … } } }`. |
| `writes` | no | The kinds of directives the plugin appends with `zhang.write`, `{ "transaction", "open" }`. The ledger owner grants them with `allowed_writes` on the `plugin` directive; without the declaration the write functions raise `unavailable: …`, without the grant `denied: …`. |

Zhang runs the chunk when it loads the ledger, to read the name, the version, the types and an analyzer's manifest, and again for every request and every analysis. Every run starts from scratch, in a fresh interpreter: no global, no table and no upvalue survives from one request to the next, and nothing is shared between plugins.

## Requests and responses

`router` receives the request of [Router Plugins](/guides/router-plugins/#the-router-export) as a table: `request.method`, `request.path` (the part below the plugin's route), `request.query` (each key to the list of its values, `{ year = { "2024" } }`), `request.headers` (lower-case names), `request.body` and `request.body_encoding` (`"utf8"`, or `"base64"` for a body that is not text).

It returns a table:

- `status`, `200` by default;
- `headers`, a table of header names to values, optional;
- the body, in one of two ways: `json = value` encodes `value` as the body and sets `content-type: application/json; charset=utf-8` unless `headers` names a content type; `body = text` sends the string as it is, as `text/plain; charset=utf-8` unless `headers` says otherwise. A `body` holding bytes that are not UTF-8, such as an image, is sent as those bytes.

Anything that is not a table is an invalid response, which Zhang answers with status 502.

## The `zhang` API

| Call | Gives |
|---|---|
| `zhang.api_version` | The version of this API, an integer; see [API versions](#api-versions). |
| `zhang.now()` | `{ now = "2026-10-13T09:30:00+08:00", today = "2026-10-13", timezone = "Asia/Shanghai" }`, read afresh for each request. There is no `os.time`. |
| `zhang.ledger_info()` | `{ title = "Home", operating_currency = "CNY", timezone = "Asia/Shanghai" }`; `title` is `json.null` when the ledger sets no `title` option. |
| `zhang.query(bql)` | Runs a [query](/reference/query-language/) and gives what `POST /api/query` returns in `data`: `{ columns = { { name = "account", type = "str" }, … }, rows = { { cell, … }, … } }`, with the same time and size limits. |
| `zhang.query_params(bql, params)` | The same, with named parameters bound: `params` maps each `:name` of the query to a typed value made with the constructors below. Bind what a user typed instead of writing it into the query text. |
| `zhang.query_builtin(name, params)` | Runs one of the [built-in queries](/reference/builtin-queries/) by name, the BQL behind the figures the web UI shows, and gives what `zhang.query` gives. `params` holds every parameter the query declares as a plain value: a string, an integer, a boolean, a list of strings, or `json.null`; a decimal is a string (`"12.50"`) or a decimal value, a date `"YYYY-MM-DD"`. A float raises before anything is sent. An analysis runs no built-in query: there it raises. |
| `zhang.query_schema()` | The tables, columns and functions a query can use, as `GET /api/query/schema` lists them. |
| `zhang.convert(conversions)` | Converts amounts with the ledger's prices, as a query's `convert(position, currency, date)` converts them: a list of `{ amount = { number = "10.00", currency = "USD" }, target = "CNY", date = "2024-01-31", via = "USD" }` in (`via`, a cost currency to go through when no price converts directly, is optional), and for each the converted `{ number = "71.0000", currency = "CNY" }`, exact, or `json.null` when no price converts it. A number is a string, an integer or a decimal value; a float raises before anything is sent. At most 10,000 conversions a call. |
| `zhang.config(key)` | A setting as a string, or `nil`: the ledger's options, the metas of the `plugin` directive, and the keys Zhang sets (`zhang.abi`, `zhang.plugin`, `zhang.seed`), exactly as a wasm plugin receives them. |
| `zhang.emit_error(message, metas)` | Reports a problem. A request has no error list, so in a router it is only logged; answer the caller in the response instead. |
| `zhang.read_file(path)`, `zhang.list_dir(path)` | The file functions of `allowed_paths`. A router has no file access, whatever its directive grants, so in this release they raise `denied: …`. |
| `zhang.str(v)`, `zhang.int(v)`, `zhang.decimal(v)`, `zhang.bool(v)`, `zhang.date(v)`, `zhang.set(v)` | A typed query parameter. `nil` is NULL for any type. A decimal is a [decimal value](#exact-decimals), made from a string such as `"12.50"` or an integer (a Lua float is refused: `decimal must be a string or an integer, got 12.5`), a date a string `"YYYY-MM-DD"`, a set a list of strings. |
| `zhang.decimal.add(a, b)`, `.sub(a, b)`, `.neg(a)`, `.abs(a)`, `.cmp(a, b)`, `.sign(a)`, `.is_zero(a)`, `.sum(list)`, `.is_decimal(v)` | The arithmetic of [decimal values](#exact-decimals) as functions, which take strings and integers as well as decimal values: `zhang.decimal.add("12.50", "0.5")` is `13.00`, `cmp` and `sign` give `-1`, `0` or `1`, `sum({})` is `0`. |
| `zhang.write.preview(batch)` | Previews a batch of up to 50 directives to append, each a table with `kind = "transaction"` (the fields of `POST /api/transactions`) or `kind = "open"` (`date`, `account`, `commodities`, `booking_method`, `metas`): for each entry the text Zhang writes, the file it goes to, the fields refused and the errors the ledger would report, exactly as `POST /api/transactions/preview` tells them, with an `open` earlier in the batch counting for the transactions after it. `token` is set when every entry is clean. Amounts are strings: a non-integer float anywhere in the batch raises `amount `12.3` must be a string (…)` before anything is sent, and the host refuses every other number. See [writes](/developers/writing-plugins/#writes). |
| `zhang.write.commit(batch, token)` | Appends the previewed batch with its token, one write per file, and gives `{ file, bytes, entries, generation }` once the text is in the files; the ledger reloads after the request. Call it as the last step of a request: once it returned, the entries are recorded. A batch that differs, a token of another ledger generation (a reload or a commit happened since), an expired token (10 minutes), or a second commit before the ledger reloaded raises `conflict: …`: preview again. A file that cannot be written raises `io: …` naming what was written and what was not. |

A call the host refuses raises a Lua error with the host's message: `query error at line 1, column 8: unknown column` for a query that fails at a position, `query error: parameter :from must be a date string YYYY-MM-DD or null` for a parameter that cannot be bound, `denied: …` for a file. Uncaught, the error fails the request with status 500; wrap the call in `pcall` to answer a clean status:

```lua
local ok, result = pcall(zhang.query, request.query.bql[1])
if not ok then return { status = 400, json = { error = result } } end
```

### API versions

`zhang.api_version` is 1 in every Zhang that runs Lua scripts, so a script using the API above needs no check: routers, analyzers, `zhang.decimal` and `zhang.convert` came in one release, the first that runs Lua scripts. A later release that adds to the `zhang` table raises the version and lists what it added below; a script that uses such an addition compares the version first, for example `if zhang.api_version < 2 then return { status = 501, json = { error = "this plugin needs Lua API version 2" } } end`, and answers that clear error on an older Zhang instead of failing on a `nil` call.

| Version | Added |
|---|---|
| 1 | The API above, routers and analyzers, in the first release that runs Lua scripts. |

## Exact decimals

Lua has 64-bit integers and floats only: `0.1 + 0.2` is not `0.3`, and a float rounds a long amount. `zhang.decimal` computes with decimals exactly, as Zhang does, in the interpreter:

```lua
local d = zhang.decimal
local total = d("0")
for _, row in ipairs(zhang.query("SELECT number WHERE account ~ '^Expenses:Food'").rows) do
  total = total + row[1]                          -- "12.50", "3.4": exact
end
d("0.10") + d("0.2")     --> 0.30
d("1") == d("1.00")      --> true
d("-1.00") + 1           --> 0.00, never -0.00
d("2") * 3               --> error: zhang.decimal has no multiplication: compute it in the host, with a query or zhang.convert
d(0.1)                   --> error: decimal must be a string or an integer, got 0.1
```

- **A decimal value** is made with `zhang.decimal(v)` from a string in plain notation (an optional sign, digits, and a point with digits: `"-12.50"`, `"+.5"`), an integer, or another decimal value. Anything else raises: a float, an exponent (`"1e3"`), spaces, a comma, and a number of more than 1000 digits. A query result gives every decimal as such a string.
- **Adding and subtracting** with `+` and `-` (and `-v`), or `zhang.decimal.add` and `.sub`, is exact, and keeps the larger scale of the two, as Zhang's own sums do: `"1.5" + "2.25"` is `3.75`, `"1.50" - "1.5"` is `0.00`, and no result is `-0`. One operand may be a string or an integer: `total + "12.50"`. A zero keeps its scale too: `"0.00" + "500"` is `500.00`, as in Python's decimal and beancount.
- **A known difference from today's budget figures.** Zhang's native budget code adds some of its figures in a way that drops the scale of a zero operand, so where a budget's `available` is `0.00` and the next month adds `500`, its `assigned` reads `500` where `zhang.decimal` gives `500.00`. The two differ only when a zero has more decimals than the other operand. `zhang.decimal` follows Zhang's documented rule; the native budget figures will be aligned with it.
- **Comparing** with `<`, `<=`, `>` and `>=` (a string or an integer on either side), `cmp`, `sign` and `is_zero` is by value: `d("1") <= "1.00"`. `==` is true only between two decimal values of the same value: `d("1") == "1"` is false, as Lua's `==` is between values of two types.
- **No multiplication, no division.** `*`, `/`, `//`, `%` and `^` raise. Zhang's products and quotients have rounding and scale rules of their own: let a query compute them (`sum`, `convert`, `*` in BQL), or `zhang.convert`.
- **Its text** is what Zhang writes: `tostring(v)`, `..`, `json.encode` and a response's `json` give `"12.50"`, plain notation with its scale. A decimal value is a string wherever JSON takes it: a cell of an analyzer's table, an amount's `number`, an amount of a write, a diagnostic's meta. As a parameter of `zhang.query_params` it is a typed decimal; `nil` is the NULL decimal parameter, which no arithmetic takes.
- **A value never changes**: `v.value` is its text, and its other fields are what the arithmetic reads. Leave them alone; make a new value instead.
- **Two strings are not decimals**: `"12.50" + "1"` is Lua's own arithmetic, in floats (`13.5`). Make one of them a decimal value.
- **Cost**: in the interpreter, about 1 µs an addition of amounts and 0.3 µs a comparison; a number of more than 18 digits takes a few µs. Let a query add up what it can: `sum(number)` adds thousands of postings in Zhang in the time a script adds a few.

## Analyzers

:::caution[Experimental]
The `Analyzer` type is experimental, in Lua as with the Rust SDK: until the budget migration of [RFC #698](/developers/rfc-698-budget-plugin/) is complete, the manifest, the records and the output may change in any release.
:::

An analyzer reads the ledger once it is loaded, at every load and so at every write, and publishes diagnostics, which join the ledger's errors, and the rows of tables that queries read like Zhang's own. [Writing Plugins](/developers/writing-plugins/#analyzers) describes the type: its manifest, the records it reads, its output, its limits. In Lua the manifest is fields of the module table, and `analyze` is a function that gets the records and returns the output, as Lua tables:

```lua
-- plugins/shop.lua: custom "shop" "<item>" "<account>" names the account of an item; the table #shop_items
-- gives the balance of each item's account, in the operating currency
local d = zhang.decimal
local M = {
  name = "shop", version = "0.1.0", types = { "Analyzer" },
  analysis_abi = 1,
  consumes = { "custom:shop" },
  diagnostics = { { kind = "Unpriced", phase = "after_fold" } },
  tables = {
    { name = "items", kind = "records", order = { "item" },
      columns = { { name = "item", type = "str" }, { name = "balance", type = "amount" } } },
  },
}

function M.analyze(input)
  local currency = input.ledger.operating_currency
  local output = { diagnostics = {}, tables = { items = {} } }
  for _, record in ipairs(input.records) do
    local item, account = record.fields.values[1], record.fields.values[2]
    -- the query adds up, in Zhang; the script converts and adds the few sums it gets
    local sums = zhang.query_params(
      "SELECT currency, sum(number) AS total WHERE account = :account GROUP BY currency ORDER BY currency",
      { account = zhang.str(account) })
    local conversions = {}
    for i, row in ipairs(sums.rows) do
      conversions[i] = { amount = { number = row[2], currency = row[1] }, target = currency, date = record.date }
    end
    local balance = d(0)
    for i, converted in ipairs(zhang.convert(conversions)) do
      if converted == json.null then
        table.insert(output.diagnostics, { kind = "Unpriced", target = { seq = record.seq },
                                           metas = { currency = sums.rows[i][1] } })
      else
        balance = balance + converted.number
      end
    end
    table.insert(output.tables.items, { item = item, balance = { number = balance, currency = currency } })
  end
  table.sort(output.tables.items, function(a, b) return a.item < b.item end)
  return output
end

return M
```

`SELECT item, balance FROM #shop_items` then reads the table, and an amount no price converts is a [`PluginError`](/reference/error-codes/#pluginerror) of the kind `Unpriced` on its `custom` directive.

- **What `analyze` gets**: the records of the kinds it consumes, in the order Zhang processed the ledger, and the ledger's settings: `input.records[i]` with `kind`, `seq`, `date`, `time`, `has_time`, `timestamp`, `fields` (`fields.values` for a `custom` directive), `meta`, `file` and `line`, and `input.ledger` with `title`, `operating_currency` and `timezone`.
- **What it reads**: the ledger through `zhang.query`, `zhang.query_params` and `zhang.query_schema`, each query with the limits of `POST /api/query`, at most 10,000 of them; `zhang.ledger_info`; `zhang.convert`; and `zhang.now`, the time of the load, which makes the ledger depend on the date. `#errors` and the `errors` and `balanced` columns are refused while it analyses, and so are its own tables. It reads no file, reaches no network and writes nothing: `zhang.query_builtin`, `zhang.read_file`, `zhang.list_dir` and `zhang.write` raise, and what it reports with `zhang.emit_error` is only logged.
- **What it returns**: a table of the shape of the output, `diagnostics`, `tables`, `series` and `labels`. A cell is a string, an integer, a boolean, a list of strings for a set, a decimal value or a string for a decimal, `{ number = …, currency = … }` (or `{ "12.50", "CNY" }`) for an amount, or `json.null`; a float in a decimal or an amount is refused. A row of a record table may be the list of its cells in the order of the columns, then maybe its metadata, which is shorter to write and to read. Where the output has a map, Lua's empty `{}` is written as an object, `json.object()` is not needed: the output itself, `tables`, `series`, `labels`, a row and its `$meta`, a partition's `key`, a label's `params`, a diagnostic's `target` and `metas`. A series partition's `until` is a Lua keyword: write `["until"] = "2024-11-01"`.
- **When it fails**: an error it raises (uncaught, with the script's line), no `analyze` function, an output that does not hold, its `timeout` or its 64 MiB of Lua values. The ledger still loads, with one [`PluginAnalysisFailed`](/reference/error-codes/#pluginanalysisfailed) error on the plugin's directive whose `reason` is `trap`, `output`, `timeout` or `memory` and whose `message` says what; a query of its tables answers that they are unavailable. A manifest that does not hold fails every analysis with the reason `manifest`.
- **What it costs**: every load and every write analyses the ledger again, so the script runs on the path of every `POST /api/transactions`. Ask few questions, each aggregated in Zhang (`GROUP BY`, `sum`, `convert`), rather than reading postings one by one, and add up only what the queries give.

## JSON

The `json` table is a JSON codec built into the interpreter, and the same rules apply to the `json` field of a response:

- `json.encode(value)` and `json.decode(text)`. `json.decode` reads standard JSON ([RFC 8259](https://www.rfc-editor.org/rfc/rfc8259)) and nothing else: no comments, no trailing commas, nothing after the value but whitespace. It raises on anything else, naming the position in bytes: `invalid JSON: missing colon at line 3, column 9`, `invalid JSON: unexpected data at byte 14`.
- **`null`.** JSON `null` decodes to `json.null`, not to `nil`, so a null field is still there (`body.payee == json.null`). Encode `json.null` to send a null. A field holding `nil` is absent.
- **Arrays and objects.** A Lua table has no kind, so an empty `{}` encodes as `[]`. A table whose keys are 1 to n is an array, with `null` for a hole, unless it is sparse (its largest key is past 10 and more than twice the number of keys); any other table is an object, with number keys written as strings. Use `json.object()`, or `json.object(t)`, for a table that must encode as an object even when it is empty or keyed by numbers, and `json.array()` for one that must encode as `[]`. What `json.decode` and the host give you keeps its kind: an object stays an object through a round trip, empty or not. A response's `headers` is always an object.
- **Marked tables are sealed.** A table from `json.decode`, the host, `json.object()` or `json.array()` carries a protected metatable: `getmetatable(t).__jsontype` reads `"object"` or `"array"` and cannot be changed, and `setmetatable(t, mt)` raises `cannot change a protected metatable`, where earlier releases allowed it. To give decoded data methods or defaults, copy it first: `local row = setmetatable({}, { __index = defaults }); for k, v in pairs(json.decode(text)) do row[k] = v end`.
- **Decimal values.** A [decimal value](#exact-decimals) is written as its text, `"12.50"`: to JSON it is a string. `json.decode` never makes one: a decoded amount is a string, which `zhang.decimal` reads.
- **Numbers.** An integer is exact within 64 bits, both ways. A float is written with 15 significant digits when they read back as the same float, else 16, else 17, so it always reads back exactly (`0.1`, `0.30000000000000004`, `1.0`); NaN and the infinities, which JSON cannot write, are written as `null`. When decoding, a number with a fraction or an exponent, or an integer past 64 bits, becomes the nearest float (`math.type` tells it apart), and one past the range of a float, such as `1e400`, raises. **Amounts never travel as numbers**: a query result gives every decimal as a string (`"12.50"`), and a plugin keeps it as a string or a decimal value. Do not `tonumber` an amount to add it up: let the query sum it, or add with [`zhang.decimal`](#exact-decimals).
- **Strings** are bytes. A NUL byte inside a string survives a round trip. A `json` value must be UTF-8 text: `json.encode` raises on a string that is not, naming the first bad byte, and `json.decode` on a document that is not; send other bytes in `body`. Control characters, U+2028, U+2029 and a few other invisible characters are written as `\u` escapes.
- **Keys are sorted.** `json.encode`, and the `json` field of a response, write an object's keys in order: number keys first, ascending, then string keys, ascending by byte. The same value always gives byte-identical JSON. `pairs` itself visits a table's keys in no fixed order, and the order changes from one request to the next: the interpreter seeds its string hash at random for every request, so keys cannot be crafted to collide. Do not rely on the order of `pairs`; sort the keys when the order matters. A table holding a number key and the same key as a string, such as `[1]` and `["1"]`, raises: JSON would write the key twice.
- **Nesting.** Arrays and objects nest up to 1000 levels; `json.encode` and `json.decode` raise past that, and `json.encode` raises `reference cycle` on a table that contains itself. The codec's buffers and the values it builds count against the [memory limit](#limits).

## The sandbox

A script has `string`, `table`, `math`, `utf8`, `coroutine`, the base functions (`pairs`, `ipairs`, `pcall`, `tostring`, `tonumber`, `select`, `setmetatable`, …), `load` for text chunks, `print` and `debug.traceback`. A script has no standard output: `print` writes one line per call to the `zhang serve` log at the info level, prefixed with the script's path (`plugins/food.lua: hello\t42`; a tab, a line break or another control character is written escaped, so a script cannot forge a line of the log), which is where a plugin author looks while developing. It has no `io`, no `os`, no `require` and no `package`, no `dofile` and no `loadfile`, no binary chunks and nothing else of `debug`. A Lua plugin cannot make HTTP requests in this release, even with `allowed_hosts`.

Each request runs in a fresh interpreter, so a script cannot keep state between requests or watch another plugin. It cannot read files (a router never can), write files, or reach the network; it reads the ledger through the API above only.

## Limits

| Limit | Value | When it is reached |
|---|---|---|
| Script size | 1 MiB, UTF-8 text | The ledger does not load. |
| Memory | 64 MiB of Lua values per request | The request fails with status 500 and the log says `plugin \`plugins/food.lua\` exceeded 64 MiB`. |
| JSON nesting | 1000 levels of arrays and objects | `json.encode` or `json.decode` raises. |
| Time | The directive's `timeout` (60 seconds by default), at most the operator's ceiling for a request, `ZHANG_PLUGIN_MAX_TIMEOUT` (30 seconds by default) | Status 504. |
| Queries | The limits of `POST /api/query`; at most 10,000 in an analysis | `zhang.query` raises. |
| Analysis | The directive's `timeout`, within what the analyses of the load before it left of `ZHANG_PLUGIN_MAX_TIMEOUT`; the output's limits of [Writing Plugins](/developers/writing-plugins/#analyze) | The ledger loads, with a `PluginAnalysisFailed` error. |

## Errors

| What happened | What Zhang does |
|---|---|
| The script does not parse, returns no module table, or declares a type other than `Router` and `Analyzer` | The ledger does not load, with the error naming the script and the line (`plugins/food.lua:12: '=' expected near 'end'`). Under `zhang serve`, the previous version of the ledger keeps being served. |
| `router` raises an error (`error()`, or a host call that failed and was not caught) | Status 500. The message, with the script's line and a traceback, goes to Zhang's log, not to the client. A host call that fails raises `kind: message`, such as `denied: …` or `conflict: …`; `pcall` it to answer the caller yourself. |
| `router` returns something that is not a table | Status 502. |
| The request runs past the `timeout` | Status 504. |
| The request allocates more than 64 MiB | Status 500, with the message above in the log. |
| `analyze` raises, returns what is no output, runs past its `timeout` or allocates more than 64 MiB | The ledger loads, with one `PluginAnalysisFailed` error on the plugin's directive saying why, the script's line included; its tables are unavailable until a load analyses it. |

## No processors or mappers yet

A Lua script may declare `Router` and `Analyzer`. A script declaring `Processor` or `Mapper` does not load, with the message *Lua processors and mappers are not supported yet (#698 P1): zhang.decimal adds, subtracts and compares but does not multiply or divide, and the host would take back an amount turned into a JSON number*.

How the directives travel is not the problem: a processor or mapper would receive them as JSON in which every decimal (an amount, a price, a cost, a tolerance) is a string, `"1234.56789012345678"`, as in a query result. Two things are:

- **No exact multiplication or division.** `zhang.decimal` adds, subtracts and compares exactly, but a stage that splits or converts an amount multiplies and divides, where Zhang rounds and scales by rules of its own. A script would fall back to floats, where `0.1 * 3` is not `0.3`.
- **Numbers are taken back.** When the host reads the directives a stage returns, an amount may be a string or a JSON number. A script that wrote a computed float into an amount would hand back a number the host accepts, and a balance would change without an error, in the stage that produces the booked ledger.

A router and an analyzer have neither problem: a router reads query results and answers HTTP, an analyzer publishes tables and diagnostics whose decimals the host refuses as JSON numbers, and nothing either returns feeds the ledger. Processors and mappers in Lua will come with a full exact-decimal plugin API, planned in [#698](https://github.com/zhang-accounting/zhang/issues/698).

## Where the interpreter comes from

The interpreter is [Lua 5.4.8](https://www.lua.org/) as built by [lua.wasm](https://github.com/andy-emerson/lua.wasm), with a JSON codec in C written for it, compiled to one WebAssembly module and embedded in the Zhang binary. Its sources, vendored with their licenses, and its build script are in `tools/lua-runtime` of the repository; the module is vendored with its checksum, and Zhang's CI rebuilds it and fails unless the bytes match. Zhang pairs it with your script at load time, so a Lua plugin is a wasm plugin as far as the sandbox, the capabilities and the route are concerned.
