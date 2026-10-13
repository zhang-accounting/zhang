---
title: Lua 插件
description: 用一个 Lua 脚本编写 router 插件，无需编译。模块表、zhang 和 json API、JSON 规则、沙箱、限制与错误。
sidebar:
  order: 9
---

插件可以是一个 Lua 脚本，而不必是 WebAssembly 模块。把 `.lua` 文件复制进账本目录，像声明任何插件一样声明它；张记账用内置的 Lua 5.4 解释器运行它，沙箱、能力和 `/api/plugins/{name}` 路由都与 wasm 插件相同。不需要编译，也不需要安装任何东西。

本版本中 Lua 脚本是 **router 插件**：它们提供页面和数据，并通过查询读取账本。[为什么只有 router](#本版本只支持-router) 见下文。

## 二十行写一个插件

```zhang
option "features.plugin" "true"

plugin "plugins/food.lua"
```

```lua
-- plugins/food.lua：本月餐饮账户的余额，以 JSON 返回
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

`GET /api/plugins/food/` 返回 JSON，`GET /api/plugins/food/?format=text` 返回账户名列表。在 `zhang serve` 下编辑 `plugins/food.lua` 会重新加载账本，和编辑 wasm 模块一样。

## 模块表

脚本是一个返回表的代码块：

| 字段 | 必填 | 含义 |
|---|---|---|
| `name` | 是 | 插件名，`GET /api/plugins` 列出的名字，也是路由 `/api/plugins/{name}` 中的名字。两个插件同名时，先声明的那个提供路由。 |
| `version` | 是 | 插件版本，一个字符串。 |
| `types` | 否 | 插件类型列表。`{ "Router" }` 是唯一可用的值，也是默认值。 |
| `router` | router 需要 | 响应请求的函数。 |
| `writes` | 否 | 插件用 `zhang.write` 追加的指令种类，`{ "transaction", "open" }`。账本所有者在 `plugin` 指令上用 `allowed_writes` 授予它们；没有声明时写入函数抛出 `unavailable: …`，没有授予时抛出 `denied: …`。 |

张记账加载账本时运行这个代码块以读取名字、版本和类型，之后每个请求再运行一次。每次运行都从头开始，在一个全新的解释器里：全局变量、表和 upvalue 都不会从一个请求残留到下一个请求，插件之间也不共享任何东西。

## 请求与响应

`router` 收到的是 [Router 插件](/zh-cn/guides/router-plugins/#router-导出函数) 中的请求，以表的形式：`request.method`、`request.path`（插件路由之下的部分）、`request.query`（每个键对应它的值列表，`{ year = { "2024" } }`）、`request.headers`（小写的名字）、`request.body` 和 `request.body_encoding`（`"utf8"`，或者非文本请求体的 `"base64"`）。

它返回一个表：

- `status`，默认 `200`；
- `headers`，可选，响应头名字到值的表；
- 响应体，二选一：`json = value` 把 `value` 编码为响应体，并在 `headers` 没有指定内容类型时设置 `content-type: application/json; charset=utf-8`；`body = text` 原样发送字符串，`headers` 没有说明时按 `text/plain; charset=utf-8` 发送。`body` 中不是 UTF-8 的字节（例如一张图片）会按原字节发送。

返回表以外的任何值都是无效响应，张记账以状态 502 回应。

## `zhang` API

| 调用 | 结果 |
|---|---|
| `zhang.now()` | `{ now = "2026-10-13T09:30:00+08:00", today = "2026-10-13", timezone = "Asia/Shanghai" }`，每个请求重新读取。没有 `os.time`。 |
| `zhang.ledger_info()` | `{ title = "Home", operating_currency = "CNY", timezone = "Asia/Shanghai" }`；账本没有设置 `title` 选项时 `title` 是 `json.null`。 |
| `zhang.query(bql)` | 运行一条[查询](/zh-cn/reference/query-language/)，给出 `POST /api/query` 在 `data` 中返回的内容：`{ columns = { { name = "account", type = "str" }, … }, rows = { { cell, … }, … } }`，时间和结果大小的限制相同。 |
| `zhang.query_params(bql, params)` | 同上，但绑定命名参数：`params` 把查询中的每个 `:name` 映射到用下面的构造函数创建的带类型的值。绑定用户输入，而不要把它拼进查询文本。 |
| `zhang.query_schema()` | 查询可以使用的表、列和函数，与 `GET /api/query/schema` 列出的相同。 |
| `zhang.config(key)` | 一项设置的字符串值，或 `nil`：账本的选项、`plugin` 指令的元数据，以及张记账设置的键（`zhang.abi`、`zhang.plugin`、`zhang.seed`），与 wasm 插件收到的完全一样。 |
| `zhang.emit_error(message, metas)` | 报告一个问题。请求没有错误列表，所以在 router 中它只会被记录到日志；要告诉调用方，请写进响应。 |
| `zhang.read_file(path)`、`zhang.list_dir(path)` | `allowed_paths` 对应的文件函数。无论指令授予什么，router 都没有文件访问权限，所以本版本中它们会抛出 `denied: …`。 |
| `zhang.str(v)`、`zhang.int(v)`、`zhang.decimal(v)`、`zhang.bool(v)`、`zhang.date(v)`、`zhang.set(v)` | 一个带类型的查询参数。任何类型的 `nil` 都是 NULL。decimal 是 `"12.50"` 这样的字符串，date 是 `"YYYY-MM-DD"` 字符串，set 是字符串列表。 |
| `zhang.write.preview(batch)` | 预览一批最多 50 条要追加的指令，每条是一个表，`kind = "transaction"`（字段与 `POST /api/transactions` 相同）或 `kind = "open"`（`date`、`account`、`commodities`、`booking_method`、`metas`）：对每个条目给出张记账写出的文本、它要写入的文件、被拒绝的字段和账本会报告的错误，与 `POST /api/transactions/preview` 完全一致；批次中靠前的 `open` 对它之后的交易生效。每个条目都没有问题时带有 `token`。金额是字符串：批次中任何位置出现非整数浮点数，在发送之前就抛出 `amount `12.3` must be a string (…)`，其他数字由宿主拒绝。见[写入](/zh-cn/developers/writing-plugins/#写入)。 |
| `zhang.write.commit(batch, token)` | 用令牌提交预览过的批次，一次写入，文本进入文件后返回 `{ file, bytes, entries, generation }`；请求结束后账本重新加载。批次有差异、令牌属于另一个账本代（期间发生了重新加载或提交）、令牌过期（10 分钟）、或在账本重新加载之前再次提交，都抛出 `conflict: …`：重新预览即可。 |

被宿主拒绝的调用会抛出带宿主消息的 Lua 错误：在某个位置失败的查询是 `query error at line 1, column 8: unknown column`，无法绑定的参数是 `query error: parameter :from must be a date string YYYY-MM-DD or null`，文件是 `denied: …`。不捕获的话，错误让请求以状态 500 失败；用 `pcall` 包住调用就能返回一个干净的状态：

```lua
local ok, result = pcall(zhang.query, request.query.bql[1])
if not ok then return { status = 400, json = { error = result } } end
```

## JSON

`json` 表是 [dkjson](https://dkolf.de/dkjson-lua/)，并固定了几条规则；响应的 `json` 字段遵循同样的规则：

- `json.encode(value)` 和 `json.decode(text)`。`json.decode` 遇到不是 JSON 的文档会抛出错误并指出位置。
- **`null`。** JSON 的 `null` 解码为 `json.null`，而不是 `nil`，所以为 null 的字段仍然存在（`body.payee == json.null`）。要发送 null，编码 `json.null`。值为 `nil` 的字段会缺失。
- **空表。** Lua 的表没有种类之分，所以空的 `{}` 会编码成 `[]`。需要即使为空也编码成 `{}` 的表，用 `json.object()` 或 `json.object(t)`；必须编码成 `[]` 的用 `json.array()`。`json.decode` 和宿主给你的值会保留种类：空对象在往返中仍是 `{}`。响应的 `headers` 永远是对象。
- **数字。** 整数在 2<sup>63</sup> 以内精确；浮点数以 14 位有效数字写出。**金额从不以数字传递**：查询结果中的每个小数都是字符串（`"12.50"`），插件也应保持为字符串。不要对金额 `tonumber` 后再相加，让查询来求和。
- **字符串**是字节。字符串内的 NUL 字节在往返中保留。`json` 值必须是 UTF-8 文本；其他字节请放在 `body` 中发送。
- **键的顺序是稳定的。** 解释器的字符串哈希种子是固定的，所以 `pairs` 在每个请求中都以相同的顺序遍历表的键，相同的请求得到字节完全相同的 JSON。这个顺序不是字母序；需要字母序的客户端请自行排序。

## 沙箱

脚本拥有 `string`、`table`、`math`、`utf8`、`coroutine`、基础函数（`pairs`、`ipairs`、`pcall`、`tostring`、`tonumber`、`select`、`setmetatable` 等）、只加载文本代码块的 `load`、`print` 和 `debug.traceback`。脚本没有标准输出：`print` 每次调用向 `zhang serve` 的日志写入一行，级别为 info，并以脚本路径作为前缀（`plugins/food.lua: hello\t42`），开发时在那里查看。它没有 `io`、没有 `os`、没有 `require` 和 `package`、没有 `dofile` 和 `loadfile`、不能加载二进制代码块，`debug` 的其他部分也没有。本版本中 Lua 插件不能发起 HTTP 请求，即使有 `allowed_hosts` 也不行。

每个请求都在全新的解释器中运行，所以脚本不能在请求之间保留状态，也看不到其他插件。它不能读文件（router 从来不能）、不能写文件、不能访问网络；它只通过上面的 API 读取账本。

## 限制

| 限制 | 值 | 达到时 |
|---|---|---|
| 脚本大小 | 1 MiB，UTF-8 文本 | 账本无法加载。 |
| 内存 | 每个请求 64 MiB 的 Lua 值 | 请求以状态 500 失败，日志记录 `plugin \`plugins/food.lua\` exceeded 64 MiB`。 |
| 时间 | 指令的 `timeout`，默认 60 秒 | 状态 504。 |
| 查询 | `POST /api/query` 的限制 | `zhang.query` 抛出错误。 |

## 错误

| 发生了什么 | 张记账的处理 |
|---|---|
| 脚本无法解析、没有返回模块表，或声明了 `Router` 以外的类型 | 账本无法加载，错误指出脚本和行号（`plugins/food.lua:12: '=' expected near 'end'`）。在 `zhang serve` 下，继续提供上一个版本的账本。 |
| `router` 抛出错误（`error()`，或没有捕获的失败宿主调用） | 状态 500。消息连同脚本行号和调用栈进入张记账的日志，不会发给客户端。失败的宿主调用抛出 `kind: message`，例如 `denied: …` 或 `conflict: …`；用 `pcall` 捕获后可以自己答复调用方。 |
| `router` 返回的不是表 | 状态 502。 |
| 请求超过 `timeout` | 状态 504。 |
| 请求分配超过 64 MiB | 状态 500，日志中有上面的消息。 |

## 本版本只支持 router

Lua 脚本只能声明 `types = { "Router" }`。声明 `Processor` 或 `Mapper` 的脚本不会加载，消息是 *Lua processors and mappers are not supported yet (#698 P1): a directive's amounts cross as JSON numbers and Lua would read them as floats*。

原因是精确性。processor 或 mapper 收到的是 JSON 形式的账本指令，其中的金额是裸的小数。Lua 没有十进制类型：它会把 `1234.56789012345678` 读成浮点数，再悄无声息地以改变后的值写回，而这正是生成已记账账本的那个阶段。router 从不接触这条指令流：它只看到查询结果，其中每个小数已经是字符串，并且它返回的任何内容都不会进入账本。Lua 的 processor 和 mapper 会随精确十进制的插件 API 一起到来，计划见 [#698](https://github.com/zhang-accounting/zhang/issues/698)。

## 解释器从哪里来

解释器是由 [lua.wasm](https://github.com/andy-emerson/lua.wasm) 构建的 [Lua 5.4.8](https://www.lua.org/)，加上 [dkjson](https://dkolf.de/dkjson-lua/)，编译成一个 WebAssembly 模块并内嵌在张记账的二进制文件中。它的源码、版本锁定和构建脚本在仓库的 `tools/lua-runtime` 目录；模块连同校验和一起提交，张记账的 CI 会重新构建它，字节不一致则失败。张记账在加载时把它和你的脚本配对，所以就沙箱、能力和路由而言，Lua 插件就是一个 wasm 插件。
