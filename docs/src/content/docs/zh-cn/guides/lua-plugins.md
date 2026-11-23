---
title: Lua 插件
description: 用一个 Lua 脚本编写 router 插件或分析器插件，无需编译。模块表、zhang 和 json API、精确小数、分析器、JSON 规则、沙箱、限制与错误。
sidebar:
  order: 9
---

插件可以是一个 Lua 脚本，而不必是 WebAssembly 模块。把 `.lua` 文件复制进账本目录，像声明任何插件一样声明它；张记账用内置的 Lua 5.4 解释器运行它，沙箱、能力和 `/api/plugins/{name}` 路由都与 wasm 插件相同。不需要编译，也不需要安装任何东西。

本版本中 Lua 脚本是 **router 插件**：它们提供页面和数据，并通过查询读取账本。它们也可以是**分析器**（实验性）：在账本加载完成后读取它，发布诊断和表（[分析器](#分析器)）。它们通过 [`zhang.decimal`](#精确小数) 精确地计算金额。[为什么还没有 processor 和 mapper](#暂不支持-processor-和-mapper) 见下文。

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
| `name` | 是 | 插件名，`GET /api/plugins` 列出的名字，也是路由 `/api/plugins/{name}` 中的名字。两个 router 插件同名时，先声明的那个提供路由，后声明的那个会在它的指令上报告一个 [`PluginError`](/zh-cn/reference/error-codes/#pluginerror)。 |
| `version` | 是 | 插件版本，一个字符串。 |
| `types` | 否 | 插件类型列表：`{ "Router" }`（默认值）、`{ "Analyzer" }`，或两者都有。没有任何类型的列表、其他类型，或者把列表写成了映射，都无法加载。 |
| `router` | router 需要 | 响应请求的函数。 |
| `analyze` | 分析器需要 | 分析账本的函数（[分析器](#分析器)）。 |
| `analysis_abi`、`namespace`、`consumes`、`open_meta`、`diagnostics`、`tables`、`columns` | 分析器需要 | 它的清单，即[清单](/zh-cn/developers/writing-plugins/#清单)中的字段，写成 Lua 表：`analysis_abi = 1`、`consumes = { "custom:shop" }`、`tables = { { name = "items", kind = "records", columns = { … } } }`。 |
| `writes` | 否 | 插件用 `zhang.write` 追加的指令种类，`{ "transaction", "open" }`。账本所有者在 `plugin` 指令上用 `allowed_writes` 授予它们；没有声明时写入函数抛出 `unavailable: …`，没有授予时抛出 `denied: …`。 |

张记账加载账本时运行这个代码块以读取名字、版本、类型和分析器的清单，之后每个请求、每次分析再运行一次。每次运行都从头开始，在一个全新的解释器里：全局变量、表和 upvalue 都不会从一个请求残留到下一个请求，插件之间也不共享任何东西。

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
| `zhang.api_version` | 本 API 的版本，一个整数；见 [API 版本](#api-版本)。 |
| `zhang.now()` | `{ now = "2026-10-13T09:30:00+08:00", today = "2026-10-13", timezone = "Asia/Shanghai" }`，每个请求重新读取。没有 `os.time`。 |
| `zhang.ledger_info()` | `{ title = "Home", operating_currency = "CNY", timezone = "Asia/Shanghai" }`；账本没有设置 `title` 选项时 `title` 是 `json.null`。 |
| `zhang.query(bql)` | 运行一条[查询](/zh-cn/reference/query-language/)，给出 `POST /api/query` 在 `data` 中返回的内容：`{ columns = { { name = "account", type = "str" }, … }, rows = { { cell, … }, … } }`，时间和结果大小的限制相同。 |
| `zhang.query_params(bql, params)` | 同上，但绑定命名参数：`params` 把查询中的每个 `:name` 映射到用下面的构造函数创建的带类型的值。绑定用户输入，而不要把它拼进查询文本。 |
| `zhang.query_builtin(name, params)` | 按名称运行一条[内置查询](/zh-cn/reference/builtin-queries/)，即网页界面显示的数字背后的 BQL，结果与 `zhang.query` 相同。`params` 以普通值给出查询声明的每个参数：字符串、整数、布尔值、字符串列表或 `json.null`；小数是字符串（`"12.50"`）或小数值，日期是 `"YYYY-MM-DD"`。浮点数在发送之前就会抛出错误。分析过程中不运行内置查询：在那里它会抛出错误。 |
| `zhang.query_schema()` | 查询可以使用的表、列和函数，与 `GET /api/query/schema` 列出的相同。 |
| `zhang.convert(conversions)` | 用账本的价格换算金额，与查询的 `convert(position, currency, date)` 换算方式相同：传入 `{ amount = { number = "10.00", currency = "USD" }, target = "CNY", date = "2024-01-31", via = "USD" }` 的列表（`via` 可选，是没有价格能直接换算时经由的成本货币），对每一项给出换算后的 `{ number = "71.0000", currency = "CNY" }`，精确无误；没有价格能换算时给出 `json.null`。数字是字符串、整数或小数值；浮点数在发送之前就会抛出错误。每次调用最多 10,000 项。 |
| `zhang.config(key)` | 一项设置的字符串值，或 `nil`：账本的选项、`plugin` 指令的元数据，以及张记账设置的键（`zhang.abi`、`zhang.plugin`、`zhang.seed`），与 wasm 插件收到的完全一样。 |
| `zhang.emit_error(message, metas)` | 报告一个问题。请求没有错误列表，所以在 router 中它只会被记录到日志；要告诉调用方，请写进响应。 |
| `zhang.read_file(path)`、`zhang.list_dir(path)` | `allowed_paths` 对应的文件函数。无论指令授予什么，router 都没有文件访问权限，所以本版本中它们会抛出 `denied: …`。 |
| `zhang.str(v)`、`zhang.int(v)`、`zhang.decimal(v)`、`zhang.bool(v)`、`zhang.date(v)`、`zhang.set(v)` | 一个带类型的查询参数。任何类型的 `nil` 都是 NULL。decimal 是一个[小数值](#精确小数)，由 `"12.50"` 这样的字符串或一个整数构造（Lua 浮点数会被拒绝：`decimal must be a string or an integer, got 12.5`），date 是 `"YYYY-MM-DD"` 字符串，set 是字符串列表。 |
| `zhang.decimal.add(a, b)`、`.sub(a, b)`、`.neg(a)`、`.abs(a)`、`.cmp(a, b)`、`.sign(a)`、`.is_zero(a)`、`.sum(list)`、`.is_decimal(v)` | [小数值](#精确小数)的运算，写成函数，除小数值外也接受字符串和整数：`zhang.decimal.add("12.50", "0.5")` 是 `13.00`，`cmp` 和 `sign` 给出 `-1`、`0` 或 `1`，`sum({})` 是 `0`。 |
| `zhang.write.preview(batch)` | 预览一批最多 50 条要追加的指令，每条是一个表，`kind = "transaction"`（字段与 `POST /api/transactions` 相同）或 `kind = "open"`（`date`、`account`、`commodities`、`booking_method`、`metas`）：对每个条目给出张记账写出的文本、它要写入的文件、被拒绝的字段和账本会报告的错误，与 `POST /api/transactions/preview` 完全一致；批次中靠前的 `open` 对它之后的交易生效。每个条目都没有问题时带有 `token`。金额是字符串：批次中任何位置出现非整数浮点数，在发送之前就抛出 `amount `12.3` must be a string (…)`，其他数字由宿主拒绝。见[写入](/zh-cn/developers/writing-plugins/#写入)。 |
| `zhang.write.commit(batch, token)` | 用令牌提交预览过的批次，每个文件写一次，文本进入文件后返回 `{ file, bytes, entries, generation }`；请求结束后账本重新加载。把它放在请求的最后一步：它返回之后条目已经记录。批次有差异、令牌属于另一个账本代（期间发生了重新加载或提交）、令牌过期（10 分钟）、或在账本重新加载之前再次提交，都抛出 `conflict: …`：重新预览即可。某个文件无法写入时抛出 `io: …`，并指明已写入和未写入的内容。 |

被宿主拒绝的调用会抛出带宿主消息的 Lua 错误：在某个位置失败的查询是 `query error at line 1, column 8: unknown column`，无法绑定的参数是 `query error: parameter :from must be a date string YYYY-MM-DD or null`，文件是 `denied: …`。不捕获的话，错误让请求以状态 500 失败；用 `pcall` 包住调用就能返回一个干净的状态：

```lua
local ok, result = pcall(zhang.query, request.query.bql[1])
if not ok then return { status = 400, json = { error = result } } end
```

### API 版本

能运行 Lua 脚本的张记账中，`zhang.api_version` 都是 1，所以只使用上面这些 API 的脚本不需要检查：router、分析器、`zhang.decimal` 和 `zhang.convert` 随同一个发布版到来，也就是第一个能运行 Lua 脚本的发布版。之后给 `zhang` 表增加内容的发布版会调高这个版本号，并在下表列出增加的内容；用到这些新增内容的脚本先比较版本，例如 `if zhang.api_version < 2 then return { status = 501, json = { error = "this plugin needs Lua API version 2" } } end`，在较旧的张记账上回答这个清楚的错误，而不是在调用 `nil` 时失败。

| 版本 | 增加了 |
|---|---|
| 1 | 上面的 API，包括 router 和分析器，随第一个能运行 Lua 脚本的发布版提供。 |

## 精确小数

Lua 只有 64 位整数和浮点数：`0.1 + 0.2` 不等于 `0.3`，浮点数还会把较长的金额舍入。`zhang.decimal` 在解释器中像张记账一样精确地计算小数：

```lua
local d = zhang.decimal
local total = d("0")
for _, row in ipairs(zhang.query("SELECT number WHERE account ~ '^Expenses:Food'").rows) do
  total = total + row[1]                          -- "12.50"、"3.4"：精确
end
d("0.10") + d("0.2")     --> 0.30
d("1") == d("1.00")      --> true
d("-1.00") + 1           --> 0.00，不会是 -0.00
d("2") * 3               --> 错误：zhang.decimal has no multiplication: compute it in the host, with a query or zhang.convert
d(0.1)                   --> 错误：decimal must be a string or an integer, got 0.1
```

- **小数值**由 `zhang.decimal(v)` 构造，`v` 是普通记法的字符串（可选的符号、数字，以及带数字的小数点：`"-12.50"`、`"+.5"`）、整数或另一个小数值。其他任何值都会抛出错误：浮点数、指数（`"1e3"`）、空格、逗号，以及超过 1000 位数字的数。查询结果给出的每个小数都是这样的字符串。
- **加和减**用 `+` 和 `-`（以及 `-v`），或 `zhang.decimal.add` 和 `.sub`，都是精确的，结果保留两者中较大的小数位数，与张记账自己的求和相同：`"1.5" + "2.25"` 是 `3.75`，`"1.50" - "1.5"` 是 `0.00`，结果从不是 `-0`。其中一个操作数可以是字符串或整数：`total + "12.50"`。零也保留它的小数位数：`"0.00" + "500"` 是 `500.00`，与 Python 的 decimal 和 beancount 相同。
- **与当前预算数字的已知差异。** 张记账原生的预算代码在计算部分数字时会丢掉为零的操作数的小数位数：某个预算的 `available` 是 `0.00`、下个月追加 `500` 时，它的 `assigned` 显示 `500`，而 `zhang.decimal` 给出 `500.00`。只有当零的小数位数多于另一个操作数时两者才不同。`zhang.decimal` 遵循张记账文档中的规则；原生预算数字之后会与之对齐。
- **比较**用 `<`、`<=`、`>` 和 `>=`（任意一边可以是字符串或整数），以及 `cmp`、`sign` 和 `is_zero`，按数值比较：`d("1") <= "1.00"`。`==` 只在两个数值相同的小数值之间为真：`d("1") == "1"` 为假，因为 Lua 的 `==` 不会比较两种类型的值。
- **没有乘法，也没有除法。** `*`、`/`、`//`、`%` 和 `^` 会抛出错误。张记账的乘积和商有自己的舍入和小数位规则：让查询来计算（BQL 中的 `sum`、`convert`、`*`），或者用 `zhang.convert`。
- **它的文本**就是张记账写出的文本：`tostring(v)`、`..`、`json.encode` 以及响应的 `json` 都给出 `"12.50"`，普通记法，保留小数位数。在 JSON 接受它的任何地方，小数值都是一个字符串：分析器表的单元格、金额的 `number`、写入的金额、诊断的元数据。作为 `zhang.query_params` 的参数，它是一个带类型的 decimal；`nil` 是 NULL 的 decimal 参数，不能参与运算。
- **小数值从不改变**：`v.value` 是它的文本，其他字段是运算读取的内容。不要改动它们；需要别的值时构造一个新的。
- **两个字符串不是小数**：`"12.50" + "1"` 是 Lua 自己的浮点数运算（`13.5`）。让其中一个操作数是小数值。
- **开销**：在解释器中，金额的一次加法约 1 µs，一次比较约 0.3 µs；超过 18 位数字的数需要几 µs。能让查询求和的就让查询求和：`sum(number)` 在张记账中把几千条记账加起来，用的时间只够脚本加几个数。

## 分析器

:::caution[实验性]
`Analyzer` 类型是实验性的，用 Lua 编写和用 Rust SDK 编写都是如此：在 [RFC #698](/zh-cn/developers/rfc-698-budget-plugin/) 的预算迁移完成之前，清单、记录和输出可能在任何发布版中改变。
:::

分析器在账本加载完成后读取它，每次加载、因而每次写入都会运行，并发布诊断（并入账本的错误）和表的行（查询读取它们就像读取张记账自己的表）。[编写插件](/zh-cn/developers/writing-plugins/#分析器)描述了这个类型：它的清单、它读取的记录、它的输出和它的限制。在 Lua 中，清单是模块表的字段，`analyze` 是一个函数，接收记录并返回输出，都以 Lua 表表示：

```lua
-- plugins/shop.lua：custom "shop" "<item>" "<account>" 指明一个商品的账户；表 #shop_items
-- 给出每个商品账户的余额，以运营货币表示
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
    -- 查询在张记账中求和；脚本只换算并累加查询给出的少数几个和
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

之后 `SELECT item, balance FROM #shop_items` 就能读取这张表；没有价格能换算的金额会成为它的 `custom` 指令上一个种类为 `Unpriced` 的 [`PluginError`](/zh-cn/reference/error-codes/#pluginerror)。

- **`analyze` 收到什么**：它消费的种类的记录，按张记账处理账本的顺序排列，以及账本的设置：`input.records[i]` 带有 `kind`、`seq`、`date`、`time`、`has_time`、`timestamp`（以及 `written_time`：夏令时切换跳过的时间按书写的时间给出，而 `time` 和 `timestamp` 把它读作一小时之后）、`fields`（`custom` 指令为 `fields.values`）、`meta`、`file` 和 `line`；`input.ledger` 带有 `title`、`operating_currency` 和 `timezone`；`input.last_transaction` 是账本的最后一笔交易（`seq` 和 `date`；没有交易时为 `nil`）。
- **它读取什么**：通过 `zhang.query`、`zhang.query_params` 和 `zhang.query_schema` 读取账本，每条查询的限制与 `POST /api/query` 相同，最多 10,000 条；`zhang.ledger_info`；`zhang.convert`；以及 `zhang.now`，即加载的时间，读取它会让账本依赖于日期。分析期间 `#errors` 以及 `errors` 和 `balanced` 列被拒绝，它自己的表也一样。它不读文件、不访问网络、不写入任何内容：`zhang.query_builtin`、`zhang.read_file`、`zhang.list_dir` 和 `zhang.write` 会抛出错误，它用 `zhang.emit_error` 报告的内容只会记录到日志。
- **它返回什么**：一个形状与输出相同的表，包含 `diagnostics`、`tables`、`series` 和 `labels`。单元格是字符串、整数、布尔值、表示集合的字符串列表、表示小数的小数值或字符串、表示金额的 `{ number = …, currency = … }`（或 `{ "12.50", "CNY" }`），或 `json.null`；小数或金额中的浮点数会被拒绝，完整写出时小数点前后合计超过 1000 位的小数也会被拒绝。记录表的一行可以写成按列顺序排列的单元格列表，后面可以再跟它的元数据，写起来和读起来都更短。输出中是映射的地方，Lua 的空 `{}` 会写成对象，不需要 `json.object()`：输出本身、`tables`、`series`、`labels`、一行及其 `$meta`、分区的 `key`、标签的 `params`、诊断的 `target` 和 `metas`。序列分区的 `until` 是 Lua 关键字：写成 `["until"] = "2024-11-01"`。
- **失败时**：它抛出的错误（未捕获，带脚本行号）、没有 `analyze` 函数、输出不成立、超过 `timeout` 或超过 64 MiB 的 Lua 值。账本仍会加载，插件的指令上有一个 [`PluginAnalysisFailed`](/zh-cn/reference/error-codes/#pluginanalysisfailed) 错误，其 `reason` 为 `trap`、`output`、`timeout` 或 `memory`，`message` 说明发生了什么；查询它的表会回答它们不可用。清单不成立时，每次分析都以 `manifest` 为原因失败。
- **它的开销**：每次加载、每次写入都会重新分析账本，所以脚本运行在每个 `POST /api/transactions` 的路径上。少提问题，每个问题都在张记账中聚合（`GROUP BY`、`sum`、`convert`），而不是逐条读取记账，脚本只累加查询给出的结果。

## JSON

`json` 表是内置在解释器中的 JSON 编解码器；响应的 `json` 字段遵循同样的规则：

- `json.encode(value)` 和 `json.decode(text)`。`json.decode` 只读取标准 JSON（[RFC 8259](https://www.rfc-editor.org/rfc/rfc8259)）：不允许注释、不允许末尾多余的逗号，值之后除空白外不能再有内容。其他输入都会抛出错误，并以字节为单位指出位置：`invalid JSON: missing colon at line 3, column 9`、`invalid JSON: unexpected data at byte 14`。
- **`null`。** JSON 的 `null` 解码为 `json.null`，而不是 `nil`，所以为 null 的字段仍然存在（`body.payee == json.null`）。要发送 null，编码 `json.null`。值为 `nil` 的字段会缺失。
- **数组和对象。** Lua 的表没有种类之分，所以空的 `{}` 会编码成 `[]`。键为 1 到 n 的表是数组，空位写成 `null`，除非它过于稀疏（最大的键超过 10，且大于键个数的两倍）；其他的表都是对象，数字键写成字符串。需要即使为空或以数字为键也编码成对象的表，用 `json.object()` 或 `json.object(t)`；必须编码成 `[]` 的用 `json.array()`。`json.decode` 和宿主给你的值会保留种类：对象在往返中仍是对象，不论是否为空。响应的 `headers` 永远是对象。
- **带标记的表是封存的。** 来自 `json.decode`、宿主、`json.object()` 或 `json.array()` 的表带有受保护的元表：`getmetatable(t).__jsontype` 读出 `"object"` 或 `"array"`，且不能修改；`setmetatable(t, mt)` 会抛出 `cannot change a protected metatable`，而之前的版本允许这样做。要给解码出的数据加上方法或默认值，先复制一份：`local row = setmetatable({}, { __index = defaults }); for k, v in pairs(json.decode(text)) do row[k] = v end`。
- **小数值。** [小数值](#精确小数)写成它的文本 `"12.50"`：对 JSON 来说它是一个字符串。`json.decode` 从不产生小数值：解码出的金额是字符串，可以交给 `zhang.decimal` 读取。
- **数字。** 整数在 64 位以内精确，编码和解码都是如此。浮点数在 15 位有效数字能读回同一个浮点数时写 15 位，否则写 16 位，再不行写 17 位，所以总能精确读回（`0.1`、`0.30000000000000004`、`1.0`）；JSON 无法表示的 NaN 和无穷大写成 `null`。解码时，带小数部分或指数的数字，以及超出 64 位的整数，会变成最接近的浮点数（用 `math.type` 可以区分），超出浮点数范围的数字（如 `1e400`）会抛出错误。**金额从不以数字传递**：查询结果中的每个小数都是字符串（`"12.50"`），插件也应保持为字符串或小数值。不要对金额 `tonumber` 后再相加：让查询来求和，或者用 [`zhang.decimal`](#精确小数) 相加。
- **字符串**是字节。字符串内的 NUL 字节在往返中保留。`json` 值必须是 UTF-8 文本：`json.encode` 遇到不是 UTF-8 的字符串会抛出错误并指出第一个错误字节，`json.decode` 遇到不是 UTF-8 的文档也会抛出错误；其他字节请放在 `body` 中发送。控制字符、U+2028、U+2029 以及其他几个不可见字符写成 `\u` 转义。
- **键是排好序的。** `json.encode` 和响应的 `json` 字段按顺序写出对象的键：先是数字键，从小到大，再是字符串键，按字节从小到大。同一个值总是得到字节完全相同的 JSON。`pairs` 本身遍历表的键没有固定顺序，而且每个请求都不一样：解释器为每个请求随机设定字符串哈希种子，这样键就无法被构造成互相冲突。不要依赖 `pairs` 的顺序；顺序重要时请自行对键排序。同一个表里既有数字键、又有写法相同的字符串键（如 `[1]` 和 `["1"]`）时会抛出错误：JSON 会把这个键写两次。
- **嵌套。** 数组和对象最多嵌套 1000 层，超过时 `json.encode` 和 `json.decode` 抛出错误；包含自身的表会让 `json.encode` 抛出 `reference cycle`。编解码器的缓冲区和它构建的值都计入[内存限制](#限制)。

## 沙箱

脚本拥有 `string`、`table`、`math`、`utf8`、`coroutine`、基础函数（`pairs`、`ipairs`、`pcall`、`tostring`、`tonumber`、`select`、`setmetatable` 等）、只加载文本代码块的 `load`、`print` 和 `debug.traceback`。脚本没有标准输出：`print` 每次调用向 `zhang serve` 的日志写入一行，级别为 info，并以脚本路径作为前缀（`plugins/food.lua: hello\t42`；制表符、换行等控制字符会被转义写出，所以脚本无法伪造日志行），开发时在那里查看。它没有 `io`、没有 `os`、没有 `require` 和 `package`、没有 `dofile` 和 `loadfile`、不能加载二进制代码块，`debug` 的其他部分也没有。本版本中 Lua 插件不能发起 HTTP 请求，即使有 `allowed_hosts` 也不行。

每个请求都在全新的解释器中运行，所以脚本不能在请求之间保留状态，也看不到其他插件。它不能读文件（router 从来不能）、不能写文件、不能访问网络；它只通过上面的 API 读取账本。

## 限制

| 限制 | 值 | 达到时 |
|---|---|---|
| 脚本大小 | 1 MiB，UTF-8 文本 | 账本无法加载。 |
| 内存 | 每个请求 64 MiB 的 Lua 值 | 请求以状态 500 失败，日志记录 `plugin \`plugins/food.lua\` exceeded 64 MiB`。 |
| JSON 嵌套 | 数组和对象共 1000 层 | `json.encode` 或 `json.decode` 抛出错误。 |
| 时间 | 指令的 `timeout`（默认 60 秒），但不超过运维者为请求设定的上限 `ZHANG_PLUGIN_MAX_TIMEOUT`（默认 30 秒） | 状态 504。 |
| 查询 | `POST /api/query` 的限制；一次分析最多 10,000 条 | `zhang.query` 抛出错误。 |
| 分析 | 指令的 `timeout`，但不超过本次加载中之前的分析用剩的 `ZHANG_PLUGIN_MAX_TIMEOUT`；输出的限制见[编写插件](/zh-cn/developers/writing-plugins/#analyze) | 账本仍会加载，带有一个 `PluginAnalysisFailed` 错误。 |

## 错误

| 发生了什么 | 张记账的处理 |
|---|---|
| 脚本无法解析、没有返回模块表，或声明了 `Router` 和 `Analyzer` 以外的类型 | 账本无法加载，错误指出脚本和行号（`plugins/food.lua:12: '=' expected near 'end'`）。在 `zhang serve` 下，继续提供上一个版本的账本。 |
| `router` 抛出错误（`error()`，或没有捕获的失败宿主调用） | 状态 500。消息连同脚本行号和调用栈进入张记账的日志，不会发给客户端。失败的宿主调用抛出 `kind: message`，例如 `denied: …` 或 `conflict: …`；用 `pcall` 捕获后可以自己答复调用方。 |
| `router` 返回的不是表 | 状态 502。 |
| 请求超过 `timeout` | 状态 504。 |
| 请求分配超过 64 MiB | 状态 500，日志中有上面的消息。 |
| `analyze` 抛出错误、返回的不是输出、超过 `timeout` 或分配超过 64 MiB | 账本仍会加载，插件的指令上有一个 `PluginAnalysisFailed` 错误说明原因，包括脚本行号；在下一次成功分析之前，它的表不可用。 |

## 暂不支持 processor 和 mapper

Lua 脚本可以声明 `Router` 和 `Analyzer`。声明 `Processor` 或 `Mapper` 的脚本不会加载，消息是 *Lua processors and mappers are not supported yet (#698 P1): zhang.decimal adds, subtracts and compares but does not multiply or divide, and the host would take back an amount turned into a JSON number*。

问题不在指令的传递方式：processor 或 mapper 收到的 JSON 中，每个小数（金额、价格、成本、容差）都是字符串，`"1234.56789012345678"`，与查询结果一样。真正的障碍有两个：

- **没有精确的乘法和除法。** `zhang.decimal` 能精确地加、减和比较，但拆分或换算金额的阶段要做乘法和除法，而张记账按自己的规则舍入和确定小数位数。脚本只能退回到浮点数，而 `0.1 * 3` 并不等于 `0.3`。
- **数字会被收回。** 宿主读取阶段返回的指令时，金额既可以是字符串，也可以是 JSON 数字。脚本若把算出的浮点数写进金额，交回的数字会被宿主接受，余额就会在生成已记账账本的阶段被悄无声息地改变，不报任何错误。

router 和分析器都没有这些问题：router 读取查询结果、回应 HTTP；分析器发布表和诊断，其中作为 JSON 数字的小数会被宿主拒绝；两者返回的任何内容都不会进入账本。Lua 的 processor 和 mapper 会随完整的精确十进制插件 API 一起到来，计划见 [#698](https://github.com/zhang-accounting/zhang/issues/698)。

## 解释器从哪里来

解释器是由 [lua.wasm](https://github.com/andy-emerson/lua.wasm) 构建的 [Lua 5.4.8](https://www.lua.org/)，加上为它编写的 C 语言 JSON 编解码器，编译成一个 WebAssembly 模块并内嵌在张记账的二进制文件中。它的源码（连同许可证一起放在仓库中）和构建脚本在仓库的 `tools/lua-runtime` 目录；模块连同校验和一起提交，张记账的 CI 会重新构建它，字节不一致则失败。张记账在加载时把它和你的脚本配对，所以就沙箱、能力和路由而言，Lua 插件就是一个 wasm 插件。
