---
title: Lua Plugins
description: Write a router plugin as a Lua script, with nothing to compile. The module table, the zhang and json API, the JSON rules, the sandbox, the limits and the errors.
sidebar:
  order: 9
---

A plugin can be a Lua script instead of a WebAssembly module. Copy a `.lua` file into your ledger and declare it like any plugin; Zhang runs it in a Lua 5.4 interpreter built into Zhang, in the same sandbox as a wasm plugin, with the same capabilities and the same `/api/plugins/{name}` route. There is nothing to compile and nothing to install.

Lua scripts are **router plugins** in this release: they serve pages and data, and read the ledger with queries. [Why only routers](#router-only-in-this-release) is explained below.

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
| `name` | yes | The plugin's name, as `GET /api/plugins` lists it and as the route `/api/plugins/{name}` names it. When two plugins share a name, the first one declared serves the route. |
| `version` | yes | The plugin's version, a string. |
| `types` | no | The plugin types, a list. `{ "Router" }` is the only value and the default; a list with no type, or a map where a list was meant, does not load. |
| `router` | for a router | The function that answers a request. |
| `writes` | no | The kinds of directives the plugin appends with `zhang.write`, `{ "transaction", "open" }`. The ledger owner grants them with `allowed_writes` on the `plugin` directive; without the declaration the write functions raise `unavailable: …`, without the grant `denied: …`. |

Zhang runs the chunk when it loads the ledger, to read the name, the version and the types, and again for every request. Every run starts from scratch, in a fresh interpreter: no global, no table and no upvalue survives from one request to the next, and nothing is shared between plugins.

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
| `zhang.now()` | `{ now = "2026-10-13T09:30:00+08:00", today = "2026-10-13", timezone = "Asia/Shanghai" }`, read afresh for each request. There is no `os.time`. |
| `zhang.ledger_info()` | `{ title = "Home", operating_currency = "CNY", timezone = "Asia/Shanghai" }`; `title` is `json.null` when the ledger sets no `title` option. |
| `zhang.query(bql)` | Runs a [query](/reference/query-language/) and gives what `POST /api/query` returns in `data`: `{ columns = { { name = "account", type = "str" }, … }, rows = { { cell, … }, … } }`, with the same time and size limits. |
| `zhang.query_params(bql, params)` | The same, with named parameters bound: `params` maps each `:name` of the query to a typed value made with the constructors below. Bind what a user typed instead of writing it into the query text. |
| `zhang.query_schema()` | The tables, columns and functions a query can use, as `GET /api/query/schema` lists them. |
| `zhang.config(key)` | A setting as a string, or `nil`: the ledger's options, the metas of the `plugin` directive, and the keys Zhang sets (`zhang.abi`, `zhang.plugin`, `zhang.seed`), exactly as a wasm plugin receives them. |
| `zhang.emit_error(message, metas)` | Reports a problem. A request has no error list, so in a router it is only logged; answer the caller in the response instead. |
| `zhang.read_file(path)`, `zhang.list_dir(path)` | The file functions of `allowed_paths`. A router has no file access, whatever its directive grants, so in this release they raise `denied: …`. |
| `zhang.str(v)`, `zhang.int(v)`, `zhang.decimal(v)`, `zhang.bool(v)`, `zhang.date(v)`, `zhang.set(v)` | A typed query parameter. `nil` is NULL for any type. A decimal is a string such as `"12.50"` or an integer (a Lua float is refused: `decimal must be a string or an integer, got 12.5`), a date a string `"YYYY-MM-DD"`, a set a list of strings. |
| `zhang.write.preview(batch)` | Previews a batch of up to 50 directives to append, each a table with `kind = "transaction"` (the fields of `POST /api/transactions`) or `kind = "open"` (`date`, `account`, `commodities`, `booking_method`, `metas`): for each entry the text Zhang writes, the file it goes to, the fields refused and the errors the ledger would report, exactly as `POST /api/transactions/preview` tells them, with an `open` earlier in the batch counting for the transactions after it. `token` is set when every entry is clean. Amounts are strings: a non-integer float anywhere in the batch raises `amount `12.3` must be a string (…)` before anything is sent, and the host refuses every other number. See [writes](/developers/writing-plugins/#writes). |
| `zhang.write.commit(batch, token)` | Appends the previewed batch with its token, one write per file, and gives `{ file, bytes, entries, generation }` once the text is in the files; the ledger reloads after the request. Call it as the last step of a request: once it returned, the entries are recorded. A batch that differs, a token of another ledger generation (a reload or a commit happened since), an expired token (10 minutes), or a second commit before the ledger reloaded raises `conflict: …`: preview again. A file that cannot be written raises `io: …` naming what was written and what was not. |

A call the host refuses raises a Lua error with the host's message: `query error at line 1, column 8: unknown column` for a query that fails at a position, `query error: parameter :from must be a date string YYYY-MM-DD or null` for a parameter that cannot be bound, `denied: …` for a file. Uncaught, the error fails the request with status 500; wrap the call in `pcall` to answer a clean status:

```lua
local ok, result = pcall(zhang.query, request.query.bql[1])
if not ok then return { status = 400, json = { error = result } } end
```

## JSON

The `json` table is [dkjson](https://dkolf.de/dkjson-lua/) with a few rules fixed, and the same rules apply to the `json` field of a response:

- `json.encode(value)` and `json.decode(text)`. `json.decode` raises on a document that is not JSON, naming the position.
- **`null`.** JSON `null` decodes to `json.null`, not to `nil`, so a null field is still there (`body.payee == json.null`). Encode `json.null` to send a null. A field holding `nil` is absent.
- **Empty tables.** A Lua table has no kind, so an empty `{}` encodes as `[]`. Use `json.object()`, or `json.object(t)`, for a table that must encode as `{}` even when empty, and `json.array()` for one that must encode as `[]`. What `json.decode` and the host give you keeps its kind: an empty object stays `{}` through a round trip. A response's `headers` is always an object.
- **Numbers.** Integers are exact up to 2<sup>63</sup>; floats are written with 14 significant digits. **Amounts never travel as numbers**: a query result gives every decimal as a string (`"12.50"`), and a plugin keeps it as a string. Do not `tonumber` an amount to add it up; let the query sum it.
- **Strings** are bytes. A NUL byte inside a string survives a round trip. A `json` value must be UTF-8 text; send other bytes in `body`.
- **Key order is stable.** The interpreter's string hash seed is fixed, so `pairs` visits a table's keys in the same order on every request and the same request gives byte-identical JSON. The order is not alphabetical; a client that needs one sorts the keys itself.

## The sandbox

A script has `string`, `table`, `math`, `utf8`, `coroutine`, the base functions (`pairs`, `ipairs`, `pcall`, `tostring`, `tonumber`, `select`, `setmetatable`, …), `load` for text chunks, `print` and `debug.traceback`. A script has no standard output: `print` writes one line per call to the `zhang serve` log at the info level, prefixed with the script's path (`plugins/food.lua: hello\t42`), which is where a plugin author looks while developing. It has no `io`, no `os`, no `require` and no `package`, no `dofile` and no `loadfile`, no binary chunks and nothing else of `debug`. A Lua plugin cannot make HTTP requests in this release, even with `allowed_hosts`.

Each request runs in a fresh interpreter, so a script cannot keep state between requests or watch another plugin. It cannot read files (a router never can), write files, or reach the network; it reads the ledger through the API above only.

## Limits

| Limit | Value | When it is reached |
|---|---|---|
| Script size | 1 MiB, UTF-8 text | The ledger does not load. |
| Memory | 64 MiB of Lua values per request | The request fails with status 500 and the log says `plugin \`plugins/food.lua\` exceeded 64 MiB`. |
| Time | The directive's `timeout`, 60 seconds by default | Status 504. |
| Queries | The limits of `POST /api/query` | `zhang.query` raises. |

## Errors

| What happened | What Zhang does |
|---|---|
| The script does not parse, returns no module table, or declares a type other than `Router` | The ledger does not load, with the error naming the script and the line (`plugins/food.lua:12: '=' expected near 'end'`). Under `zhang serve`, the previous version of the ledger keeps being served. |
| `router` raises an error (`error()`, or a host call that failed and was not caught) | Status 500. The message, with the script's line and a traceback, goes to Zhang's log, not to the client. A host call that fails raises `kind: message`, such as `denied: …` or `conflict: …`; `pcall` it to answer the caller yourself. |
| `router` returns something that is not a table | Status 502. |
| The request runs past the `timeout` | Status 504. |
| The request allocates more than 64 MiB | Status 500, with the message above in the log. |

## Router only in this release

A Lua script may declare `types = { "Router" }` only. A script declaring `Processor` or `Mapper` does not load, with the message *Lua processors and mappers are not supported yet (#698 P1): a directive's amounts cross as JSON numbers and Lua would read them as floats*.

The reason is exactness. A processor or mapper receives the ledger's directives as JSON, where amounts are bare decimals. Lua has no decimal type: it would read `1234.56789012345678` as a floating-point number and write it back changed, silently, in the stage that produces the booked ledger. A router never touches that stream: it only sees query results, where every decimal is already a string, and nothing it returns feeds the ledger. Processors and mappers in Lua will come with the exact-decimal plugin API, planned in [#698](https://github.com/zhang-accounting/zhang/issues/698).

## Where the interpreter comes from

The interpreter is [Lua 5.4.8](https://www.lua.org/) as built by [lua.wasm](https://github.com/andy-emerson/lua.wasm), with [dkjson](https://dkolf.de/dkjson-lua/), compiled to one WebAssembly module and embedded in the Zhang binary. Its source, pins and build script are in `tools/lua-runtime` of the repository; the module is vendored with its checksum, and Zhang's CI rebuilds it and fails unless the bytes match. Zhang pairs it with your script at load time, so a Lua plugin is a wasm plugin as far as the sandbox, the capabilities and the route are concerned.
