---
title: 内置查询
description: 张记账所显示的各项数字背后有文档说明的 BQL 查询，如何在查询页面打开并修改它们，以及列出这些查询、执行它们、填入参数值的 HTTP 接口。
---

张记账的页面通过*内置查询*计算各项数字（[#479](https://github.com/zhang-accounting/zhang/issues/479)）。内置查询是用张记账的[查询语言](/zh-cn/reference/query-language/)写成的具名查询，页面通过下面的 [HTTP API](#http-api)，带着几个参数（例如报表的日期）在你的账本上执行它。页面只负责排列查询结果，所以每个数字背后的逻辑都是一个可以在本页读到的查询。

你可以在**查询**页面打开某个数字背后的查询，页面当时使用的参数值已经填好，然后修改它：换一个日期范围、加入更多账户、换一种分组方式、画成图表。应用能显示的任何内容，你都可以自己查询并加以变化，无需等待新版本。

:::note[库 API 迁移]
读取端点迁移后，旧的 `Operations` 余额、账户列表、收款方列表和预算计算方法，以及 `AccountBalanceDomain`、`Store.budgets` 和对应聚合类型已删除（WASM playground 返回的序列化 Store 也不再包含这些字段）。请改用下面的查询，以及 `#budget_definitions`、`#budgets` 和 `#budget_events`。

Rust 的 `PostingDomain` 不再存储 `previous_amount` 和 `after_amount`，Python 绑定中的同名 getter 也已删除。请使用查询引擎的 [`account_balance`](/zh-cn/reference/query-language/#账户余额) 列读取一条分录后的真实账户余额。`inferred_amount` 仍表示该分录实际记入的数量。预算 HTTP 响应的事件类型仍为 `AddAssignedAmount` 和 `Transfer`；对应的 Rust 枚举现位于 `zhang_server::response`。

`GET /api/store` 已在 [#606](https://github.com/zhang-accounting/zhang/pull/606) 中单独移除。请通过公开端点或 `POST /api/query` 读取数据。
:::

## 打开查询

来自内置查询的数字旁边有 **打开查询** 操作。它会打开查询页面（`/explore`），把查询放进编辑器并执行。参数已经以值的形式写进查询，所以编辑器里的查询是完整的。你可以像对待其他查询一样修改它并再次执行，也可以用 [`query` 指令](/zh-cn/reference/directives/query/)把它保存到账本中。

## 参数

内置查询把它的输入命名为参数，写作 `:name`，例如 `WHERE date >= :from AND date <= :to`。张记账执行查询时绑定参数的值。你在页面中输入的内容（例如搜索关键字）只会作为参数绑定，绝不会被拼接进查询文本。

在查询页面打开查询时，每个参数都会被替换为它的值，写法保证查询能原样读回该值：

| 类型 | 写法 | 示例 |
|------|------|------|
| `date` | `YYYY-MM-DD` | `2024-01-31` |
| `str` | 放在单引号中；如果含有单引号，则放在双引号中 | `'Assets:Bank'`、`"O'Brien"` |
| `set` | 调用 [`set`](/zh-cn/reference/query-language/#搜索函数) | `set('food', 'trip')`、`set()` |
| `int` | 数字本身，负数加括号 | `12`、`(-3)` |
| `decimal` | 保留小数位的数字，总带小数点，负数加括号 | `12.50`、`12.`、`(-0.5)` |
| `bool` | `TRUE` 或 `FALSE` | |
| 任意类型 | 没有值时为 `NULL`，通常表示不使用该筛选条件 | `NULL` |

查询语言中的字符串没有转义序列：字符串一直延续到下一个同种引号为止，反斜杠保持原样，所以 `'C:\temp\'` 就是文本 `C:\temp\`。同时含有两种引号的文本会写成字符串的拼接，例如 `("it's " + '"quoted"')`。

写入参数值后的查询返回的行与页面得到的完全相同。只有不带 `AS` 的列名会变化，因为列名就是该列的文本。

## HTTP API

这些接口与其他 API 一样受[身份认证](/zh-cn/deployment/authentication/)保护。脚本通过它们得到的，正是页面用来计算各项数字的那些行。

### 列出内置查询

`GET /api/query/builtins` 列出所有内置查询，包括名称、说明、BQL、参数及其[类型](/zh-cn/reference/query-language/#类型)，以及它返回的行的各列：

```json
{
  "data": [
    {
      "name": "postings.between",
      "description": "Every posting between two ledger dates, both included, in ledger order.",
      "bql": "SELECT date, flag, payee, narration, account, position WHERE date >= :from AND date <= :to ORDER BY seq",
      "params": [
        { "name": "from", "type": "date" },
        { "name": "to", "type": "date" }
      ],
      "columns": [
        { "name": "date", "type": "date" },
        { "name": "flag", "type": "str" },
        { "name": "payee", "type": "str" },
        { "name": "narration", "type": "str" },
        { "name": "account", "type": "str" },
        { "name": "position", "type": "position" }
      ]
    }
  ]
}
```

### 参数值

下面的每个接口都接受查询的 `params`：按名称给出查询的每一个参数，且不能有多余的参数，每个值是对应类型的 JSON 值：

| 类型 | JSON 值 |
|------|---------|
| `date` | 字符串 `YYYY-MM-DD`，读法与 [`date(str)`](/zh-cn/reference/query-language/#日期函数) 相同 |
| `str` | 字符串 |
| `set` | 字符串列表 |
| `int` | 整数 |
| `decimal` | 数字，或者 `"12.50"` 这样的字符串，以保留小数位 |
| `bool` | `true` 或 `false` |
| 任意类型 | `null` |

查询名称不存在时返回 HTTP 404。参数缺失、多余或类型不对时返回 HTTP 400。`message` 中会指出是哪个查询或哪个参数。

### 执行内置查询

`POST /api/query/builtins/{name}` 把 `params` 中的值绑定到参数上执行名为 `name` 的查询，响应与 [`POST /api/query`](/zh-cn/reference/query-language/#执行查询) 相同：`columns` 就是上面列表中该查询的各列，`rows` 中每一行是按列顺序排列的单元格列表，编码见[单元格编码](/zh-cn/reference/query-language/#单元格编码)。请求中带 `"count_total": true` 时，结果还包含 `total`，即 `LIMIT` 和 `OFFSET` 之前的行数。`POST /api/query` 的[限制](/zh-cn/reference/query-language/#限制)同样适用。

```shell
curl -X POST http://localhost:8000/api/query/builtins/postings.between \
  -H 'Content-Type: application/json' \
  -d '{"params": {"from": "2024-01-01", "to": "2024-01-31"}}'
```

```json
{
  "data": {
    "columns": [
      { "name": "date", "type": "date" },
      { "name": "flag", "type": "str" },
      { "name": "payee", "type": "str" },
      { "name": "narration", "type": "str" },
      { "name": "account", "type": "str" },
      { "name": "position", "type": "position" }
    ],
    "rows": [
      ["2024-01-31", "*", "Shop", "Lunch", "Expenses:Food", { "units": { "number": "12.50", "currency": "USD" }, "cost": null }],
      ["2024-01-31", "*", "Shop", "Lunch", "Assets:Cash", { "units": { "number": "-12.50", "currency": "USD" }, "cost": null }]
    ]
  }
}
```

### 一次执行多个查询

`POST /api/query/builtins` 执行一组查询，每一项都是上面那样的 `{"name", "params", "count_total"}`，整组在账本的同一次读取中执行，因此即使账本正在重新加载，同一页面上的各项数字也彼此一致。响应是各查询结果的列表，顺序与请求相同。如果其中某个查询不存在或参数有误，整组请求都会被拒绝，返回该查询的 404 或 400。

```shell
curl -X POST http://localhost:8000/api/query/builtins \
  -H 'Content-Type: application/json' \
  -d '[{"name": "journals.payees", "params": {}},
       {"name": "postings.between", "params": {"from": "2024-01-01", "to": "2024-01-31"}, "count_total": true}]'
```

### 写出带参数值的查询

`POST /api/query/builtins/{name}/text` 返回名为 `name` 的查询，其中的参数已替换为 `params` 中的值，也就是查询页面收到的文本：

```shell
curl -X POST http://localhost:8000/api/query/builtins/postings.between/text \
  -H 'Content-Type: application/json' \
  -d '{"params": {"from": "2024-01-01", "to": "2024-01-31"}}'
```

```json
{
  "data": {
    "query": "SELECT date, flag, payee, narration, account, position WHERE date >= 2024-01-01 AND date <= 2024-01-31 ORDER BY seq"
  }
}
```

把这段文本发送到 [`POST /api/query`](/zh-cn/reference/query-language/#执行查询) 即可执行，得到的行与按名称执行该查询相同。

### 已移除的接口

Web 界面以前使用的各类型化读取接口正在按组逐个版本移除，改为按名称执行内置查询：以前调用这些接口的脚本，用接口所执行的查询、绑定接口当时使用的参数，就能得到相同的数字。已移除的接口与其他没有路由的 `/api` 路径一样，返回 HTTP 404 和指明该路径的 JSON `message`（如 `no route GET /api/budgets`），无论构建是否带有 Web 界面；路径上仍有其他方法时（如 `GET /api/accounts/{account}/documents`，其 `POST` 用于上传），返回 HTTP 405 和指明该方法与路径的消息。

| 已移除的接口 | 改为执行的内置查询 |
|--------------|--------------------|
| `GET /api/budgets?year=&month=` | `budgets.month`，`month` 为该月的第一天 |
| `GET /api/budgets/{name}?year=&month=` | `budgets.budget`（参数 `name`）和 `budgets.budget_month`（参数 `name`、`month`）；`budgets.budget` 没有行即原来的 404，`budgets.budget_month` 没有行表示预算开始之前的月份（未分配、未支出、未关闭） |
| `GET /api/budgets/{name}/interval/{year}/{month}` | `budgets.events` 和 `budgets.postings`，参数均为 `name`、`month`；原接口把两个列表按时间从新到旧合并，时间相同时预算自身的条目在前 |
| `GET /api/documents` | `journals.documents`；原接口附加的文件名、扩展名和据此猜测的 MIME 类型都可以从行的 `path` 得出 |
| `GET /api/accounts/{account}/documents` | `accounts.documents`，参数 `account`，其余同上 |
| `GET /api/accounts/{account}/balances` | `accounts.balance_history`，参数 `account`；原接口按 `currency` 把行分组 |
| `GET /api/statistic/summary?from=&to=` | `report.net_worth` 和 `report.liabilities`（参数 `to`、`currency`，即运营货币），`report.flows`（`from`、`to`、`currency`），`report.transaction_count`（`from`、`to`）；原接口的 `calculated` 是行的 `value` 库存中 `currency` 的部分，`detail` 是 `units` 库存按货币的数量 |
| `GET /api/statistic/{type}?from=&to=` | `report.account_totals` 和 `report.top_postings`（参数 `type`、`from`、`to`、`currency`）；最大分录行的 `date`、`time`、`timestamp`、`account`、`id`、`payee`、`narration`、`units`、`account_balance` 对应原接口的 `datetime`、`trx_id`、`inferred_unit`、`account_after` |
| `GET /api/errors?page=&size=` | `journals.errors`，参数 `size` 和 `offset = (page - 1) * size`，加 `count_total` 以计算页数；行的 `kind`、`file`、`line`、`column`、`span_start`、`span_end`、`source` 和 `metas`（`{key, value}` 列表）对应原接口的 `error_type`、`span.filename`、`span.line`、`span.column`、`span.start`、`span.end`、`span.content` 和 `metas` |

原接口的金额形如 `{number, commodity}`；查询的单元格形如 `{number, currency}`，预算的 `activity` 是以该预算 `currency` 计的数字。

## 查询列表

### 通用

#### `postings.between`

两个账本日期之间（含首尾两天）的所有分录，按账本顺序排列。

| 参数 | 类型 | 值 |
|------|------|----|
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |

```sql
SELECT date, flag, payee, narration, account, position
WHERE date >= :from AND date <= :to
ORDER BY seq
```

#### `postings.matching`

某个收款人、带有某些标签之一的交易的分录，按账本顺序排列；参数为 NULL 时不使用对应的筛选条件。

| 参数 | 类型 | 值 |
|------|------|----|
| `payee` | `str` | 收款人；`NULL` 表示任意 |
| `tags` | `set` | 交易须带有其中任一标签；`NULL` 表示任意 |

```sql
SELECT date, payee, narration, tags, account, position
WHERE (:payee IS NULL OR payee = :payee) AND (:tags IS NULL OR intersects(tags, :tags))
ORDER BY seq
```

#### `ledger.now`

账本时钟在账本时区中的当前日期和时刻，即 [`today()`](/zh-cn/reference/query-language/#日期函数) 和 `now()` 的值，一行。各表单以此作为所写内容的默认日期和时间，并把它们绑定到 `journals.accounts`，因此表单提供的账户正是该瞬间开立的账户。没有任何账户的账本不返回行，此时应用改用浏览器的时钟；这并无影响，因为没有账户时两个表单都无法提交任何内容。

```sql
SELECT today() AS date, now() AS time FROM #accounts LIMIT 1
```

### 报表

**报表**页和首页（汇总与排行通过这些查询，图表通过 `GET /api/statistic/graph`）。范围是两个账本日期 `from` 和 `to`，含首尾两天；这些接口也接受时间点，表示该时间点在账本时区中所在的那一天。三个接口都以相同的形式在 `from` 和 `to` 中返回所报告的范围：这些日期在账本时钟上的第一秒和最后一秒，不带时区，例如 `2024-06-01T00:00:00` 和 `2024-06-30T23:59:59`。`currency` 是账本的运营货币。

- **估值。**汇总和排行按 `to` 当天的价格估值。图表的每个点按它自己最后一天的价格估值，最后一个点按 `to` 的价格估值。价格可以反向使用；按成本持有、自身没有价格的持仓通过成本货币估值（见 [`convert`](/zh-cn/reference/query-language/#估值函数)）。任何价格都换算不了的金额保留原货币，不计入以运营货币表示的合计。
- **图表**每天、每周（周一到周日）或每月一个点，以它的第一天命名，因此第一周（月）和最后一周（月）可能超出范围；报表页面用范围的第一天标注第一个点。范围内没有分录的点沿用前一个点的净资产，最前面的点沿用 `from` 前一天的净资产，并按它自己最后一天的价格估值（见 `report.net_worth_trend`）。
- **限制。**图表最多 50,000 个点，约合 137 年的每日数据；按日请求更长的范围会得到 HTTP 400，请改为按周或按月。这些数字也遵守所有查询的[限制](/zh-cn/reference/query-language/#限制)：图表的点（每种货币一个值）计入结果大小限制（`ZHANG_QUERY_MAX_RESULT_VALUES`），超出它或超出时间限制的图表同样得到 HTTP 400。图表的开销随所请求的范围增长，而不随范围之前的账本历史增长。

#### `report.net_worth`

`to` 当天结束时的净资产（资产与负债的余额），按当天的价格以 `currency` 估值，即汇总中的余额。`balance` 保留批次。

| 参数 | 类型 | 值 |
|------|------|----|
| `to` | `date` | 余额的日期 |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT sum(position) AS balance, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
```

#### `report.liabilities`

`to` 当天结束时负债的余额，按当天的价格以 `currency` 估值。与账本中一样，它是负数。

| 参数 | 类型 | 值 |
|------|------|----|
| `to` | `date` | 余额的日期 |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, 'Liabilities') AND date <= :to
```

#### `report.flows`

范围内的收入和支出，按 `to` 当天的价格以 `currency` 估值。与账本中一样，收入为负数。

| 参数 | 类型 | 值 |
|------|------|----|
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT root(account, 1) AS type, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Income') OR under(account, 'Expenses')) AND date >= :from AND date <= :to
GROUP BY type
ORDER BY type
```

#### `report.transaction_count`

范围内的交易笔数。`balance ... with pad` 生成的补齐交易（标记 `P`）不计入，余额断言不是交易。

| 参数 | 类型 | 值 |
|------|------|----|
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |

```sql
SELECT count(*) AS transactions
FROM #transactions
WHERE flag != 'P' AND date >= :from AND date <= :to
```

#### `report.net_worth_trend`

范围内每个有分录的日、周或月结束时的净资产（资产与负债的余额），按它在范围内最后一天的价格以 `currency` 估值。[`OPEN ON :from`](/zh-cn/reference/query-language/#会计期间) 把 `from` 之前的一切逐批次替换为前一天的期初余额，因此累计的 `balance` 从期初余额开始，查询只对范围内的日期以及前一天所在的区间分组。`interval` 为 `'1 day'`、`'1 week'` 或 `'1 month'`：[`date_bin`](/zh-cn/reference/query-language/#日期函数) 从 2001-01-01（既是周一，又是月初）开始划分的区间，就是日历上的日、从周一开始的周和月，每个区间以它的第一天命名。[`least`](/zh-cn/reference/query-language/#比较函数) 让最后一个区间的估值日期不超出范围。`balance` 保留批次，用于给没有分录的点估值。

这个查询只列出有分录的日、周或月，因此**打开查询**显示的行比图表的点少。图表把没有分录的日、周或月补上它之前的最后一个余额（包括期初余额），并按它自己在范围内的最后一天估值，与当天的 `report.net_worth` 的估值相同。查询语言目前还不能列出没有分录的日期。

| 参数 | 类型 | 值 |
|------|------|----|
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |
| `interval` | `str` | `'1 day'`、`'1 week'` 或 `'1 month'` |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT date_bin(:interval, date, 2001-01-01) AS bucket, last(balance) AS balance, units(last(balance)) AS units,
  convert(last(balance), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
FROM OPEN ON :from
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY bucket
ORDER BY bucket
```

#### `report.changes`

范围内每日、每周或每月各账户类型的变动，按它在范围内最后一天的价格以 `currency` 估值，即收支图表中的柱。区间与 `report.net_worth_trend` 相同；第一个区间只计入 `from` 当天及以后的分录。

| 参数 | 类型 | 值 |
|------|------|----|
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |
| `interval` | `str` | `'1 day'`、`'1 week'` 或 `'1 month'` |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT date_bin(:interval, date, 2001-01-01) AS bucket, root(account, 1) AS type, units(sum(position)) AS units,
  convert(sum(position), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
WHERE date >= :from AND date <= :to
GROUP BY bucket, type
ORDER BY bucket, type
```

#### `report.account_totals`

某一类型（例如 `'Expenses'`）的每个账户在范围内的变动，按 `to` 当天的价格以 `currency` 估值，按估值从小到大排列，即收入和支出的构成。

| 参数 | 类型 | 值 |
|------|------|----|
| `type` | `str` | `'Assets'`、`'Liabilities'`、`'Equity'`、`'Income'` 或 `'Expenses'` |
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT account, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, :type) AND date >= :from AND date <= :to
GROUP BY account
ORDER BY number(only(:currency, convert(sum(position), :currency, :to))), account
```

#### `report.top_postings`

范围内某一类型账户的十笔最大分录，按 `to` 当天的价格以 `currency` 估值排序，即最大的支出和收入。[`possign`](/zh-cn/reference/query-language/#金额与数值) 把收入和负债变为正数，因此最大的收入排在最前。任何价格都无法换算为 `currency` 的分录排在最后。`account_balance` 是该分录之后其账户以分录货币计的余额。

| 参数 | 类型 | 值 |
|------|------|----|
| `type` | `str` | `'Assets'`、`'Liabilities'`、`'Equity'`、`'Income'` 或 `'Expenses'` |
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units,
  only(currency, account_balance) AS account_balance, convert(position, :currency, :to) AS value
WHERE under(account, :type) AND date >= :from AND date <= :to
ORDER BY currency(convert(position, :currency, :to)) = :currency DESC, number(possign(convert(position, :currency, :to), account)) DESC
LIMIT 10
```

### 账户

账户页面显示该账户**及其子账户**，与账户树一致：它的流水、余额历史、合计和文档都涵盖整棵子树。`Assets:Bank` 的页面包含 `Assets:Bank:Checking` 的分录，流水中的每一行都注明分录所属的账户。余额用 [`convert`](/zh-cn/reference/query-language/#估值函数) 按今天的价格折算为运营货币，它会使用反向价格和持仓的成本货币。

含子账户的余额由这些查询的行相加得到：账户列表和账户页面把一个账户及其下所有账户在 `accounts.balances` 或 `accounts.subtree_balances` 中的行相加，与网页界面的账户树一致。要自己查询一棵子树的合计，可以用 [`under`](/zh-cn/reference/query-language/#账户函数) 筛选：

```sql
SELECT currency, sum(number) AS units
WHERE under(account, 'Assets:Bank')
GROUP BY currency
```

账户页面的流水按 [`seq`](/zh-cn/reference/query-language/#处理顺序)（张记账处理账本的顺序）合并 `accounts.journal` 和 `accounts.balance_assertions` 的行，最新的在前。因此一个断言紧跟在它的余额所包含的分录之后，也就是张记账检查它的位置：写了时刻的余额断言在当天该时刻之前的交易之后，普通的余额断言在写在它之前的补齐之后，`balance ... with pad` 在同一时刻的其他余额记录之后。它的余额就是所在位置的累计余额，它的 `trx_id` 是其检查结果的 id，与 `GET /api/journals` 列出它时使用的 id 相同；分录行的 `trx_id` 是其交易的 id。同一交易的各行按分录最新的在前，按多个批次记账的一笔分录显示为一行。在夏令时跳过某段时间的那天，决定顺序的是写下的时刻，所以写在跳过时段中的行可能排在存储时刻更早的行之前。

页面按每页 100 行列出流水（`GET /api/accounts/{account}/journals?page=1&size=100`），`accounts.journal` 的每一行（一笔分录）和每个断言各算一行，所以最后一页之前的每一页都是满的。一页用 `accounts.journal_page` 从流水的末尾读取它的分录；分录的数量就是这个查询的总行数，`POST /api/query` 带上 `count_total` 时会返回它。

#### `accounts.list`

每个有 `open` 或 `close` 指令的账户，及其开户和销户日期、别名，以及由 [`account_status`](/zh-cn/reference/query-language/#账户与商品指令) 得出的、在某个日期和时间的状态，按名称排序。它和 `accounts.balances` 一起组成账户列表，账户列表按账本时钟查询当前时刻：有分录但没有 `open` 指令的账户也会列出，显示为开立。状态为 `'closed'` 的账户在列表中显示为已关闭。

| 参数 | 类型 | 值 |
|------|------|----|
| `date` | `date` | 日期，按账本时区：账户列表用今天 |
| `time` | `str` | 一天中的时间，`HH:MM:SS`：账户列表用当前时间 |

```sql
SELECT account, open, close, meta('alias') AS alias, account_status(account, :date, :time) AS status
FROM #accounts
ORDER BY account
```

#### `accounts.balances`

每个有分录的账户自身分录的余额，按货币分行：数量、按今天的价格折算为运营货币的价值，以及第一笔分录的日期。账户列表把一个账户及其下所有账户的行相加，得到含子账户的余额。

| 参数 | 类型 | 值 |
|------|------|----|
| `operating_currency` | `str` | 账本的 `operating_currency` 选项 |

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
GROUP BY account, currency
ORDER BY account, currency
```

#### `accounts.subtree`

一个账户及其有 `open` 或 `close` 指令的子账户。账户页面从中读取日期、状态和别名。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |
| `date` | `date` | 日期，按账本时区：今天 |
| `time` | `str` | 一天中的时间，`HH:MM:SS`：当前时间 |

```sql
SELECT account, open, close, meta('alias') AS alias, account_status(account, :date, :time) AS status
FROM #accounts
WHERE under(account, :account)
ORDER BY account
```

#### `accounts.subtree_balances`

一个账户及其每个子账户的余额，与 `accounts.balances` 相同。账户页面显示这些行的合计，也就是对该账户的 `balance` 断言所检查的余额，以及该账户自身的余额。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |
| `operating_currency` | `str` | 账本的 `operating_currency` 选项 |

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
WHERE under(account, :account)
GROUP BY account, currency
ORDER BY account, currency
```

#### `accounts.journal`

该账户及其子账户的分录，按账本顺序排列，每笔分录一行，每行带有该账户及其子账户紧接其后的[累计余额](/zh-cn/reference/query-language/#累计余额)，包含所有货币；页面显示其中分录货币的那一项。`balance ... with pad` 生成的补齐交易与其他交易一样列出。按多个[批次](/zh-cn/reference/query-language/#批次记账)记账的一笔分录在分录表中每个批次一行，它们的 `seq` 和 `posting_index` 相同：按这两列分组后，各批次相加得到分录的数量，`first()` 取它的日期、收付款人等列，`last()` 取最后一个批次之后的余额。加上 `ORDER BY seq DESC, posting_index DESC` 可以像页面一样把最新的排在最前。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |

```sql
SELECT first(date) AS date, first(time) AS time, first(timestamp) AS timestamp, first(flag) AS flag,
       first(id) AS id, first(account) AS account, first(payee) AS payee, first(narration) AS narration,
       seq, posting_index, sum(number) AS units, first(currency) AS currency, last(units(balance)) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index
```

#### `accounts.journal_page`

`accounts.journal` 的部分行，按账本顺序：流水的一页从末尾数起读取它的分录。由于一笔分录的各批次行前后相连，不论偏移多少，只构建这一页的分录。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |
| `limit` | `int` | 分录数 |
| `offset` | `int` | 之前跳过的分录数 |

```sql
SELECT first(date) AS date, first(time) AS time, first(timestamp) AS timestamp, first(flag) AS flag,
       first(id) AS id, first(account) AS account, first(payee) AS payee, first(narration) AS narration,
       seq, posting_index, sum(number) AS units, first(currency) AS currency, last(units(balance)) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index
LIMIT :limit OFFSET :offset
```

#### `accounts.balance_assertions`

对该账户的余额断言，按 [`seq`](/zh-cn/reference/query-language/#处理顺序) 最新的在前，列与 `journals.balance_checks` 相同。`actual` 是断言所检查的该账户及其子账户的余额，也就是流水中它的 `seq` 所在位置的累计余额。`pad` 是 `balance ... with pad` 用来补齐的账户。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |

```sql
SELECT date, time, timestamp, id, account, amount, tolerance, actual, amount - actual AS difference, passed, pad, seq
FROM #balances
WHERE account = :account
ORDER BY seq DESC
```

#### `accounts.balance_history`

账户页面的余额历史图：该账户及其子账户在每个有分录的日子结束时的余额，按货币分开，按日期排序。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |

```sql
SELECT date, currency, last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY date, currency
ORDER BY date, currency
```

#### `accounts.documents`

账户页面的文档：该账户及其子账户的 `document` 指令，按账本顺序排列。`path` 是文件相对于账本目录的路径，页面用它下载文件。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |

```sql
SELECT date, time, account, path, transaction_id
FROM #documents
WHERE source = 'directive' AND under(account, :account)
```

`document` 指令的 `transaction_id` 为 `NULL`；这一列让结果与 [`journals.documents`](/zh-cn/reference/builtin-queries/#journalsdocuments) 的列相同，两个列表以同样的方式显示文档。

### 流水

流水页面（`GET /api/journals`）、新建交易表单建议的收款人和账户（`GET /api/for-new-transaction`）、文档页面（`GET /api/documents`）以及错误列表（`GET /api/errors`）。

#### `journals.page`

流水的一页，最新的在前：符合关键词、标签和链接的交易（包括补齐交易）与余额断言；参数为 NULL 时不使用对应的筛选条件。

| 参数 | 类型 | 值 |
|------|------|----|
| `keyword` | `str` | 搜索文本；`NULL` 表示不搜索。交易的收款人、描述、标签、链接或账户包含它（不区分大小写）时匹配；余额断言的账户或 `Balance Check` 字样包含它时匹配。它是普通文本，不是正则表达式。 |
| `tags` | `set` | 交易须带有其中任一标签；`NULL` 表示任意。余额断言没有标签 |
| `links` | `set` | 交易须带有其中任一链接；`NULL` 表示任意 |
| `size` | `int` | 每页的行数，1 到 1000 |
| `offset` | `int` | 该页之前的行数：`(page - 1) × size` |

```sql
SELECT seq, type, id, date, time, flag, payee, narration, tags, links, metas
FROM #entries
WHERE (type = 'transaction'
       AND (:tags IS NULL OR intersects(tags, :tags))
       AND (:links IS NULL OR intersects(links, :links))
       AND (:keyword IS NULL OR icontains(payee, :keyword) OR icontains(narration, :keyword)
            OR any_icontains(tags, :keyword) OR any_icontains(links, :keyword) OR any_icontains(accounts, :keyword)))
   OR (type = 'balance' AND :tags IS NULL AND :links IS NULL
       AND (:keyword IS NULL OR icontains('Balance Check', :keyword) OR any_icontains(accounts, :keyword)))
ORDER BY seq DESC
LIMIT :size OFFSET :offset
```

- 页数按 `LIMIT` 和 `OFFSET` 之前的总行数计算。所有分页接口（`GET /api/journals`、`GET /api/errors` 和 `GET /api/accounts/{account}/journals`）都按同一规则读取 `page` 和 `size`：页码从 1 开始，默认第 1 页；页大小 `size` 为 1 到 1000，默认 100。页码为 0 时返回 HTTP 400，消息为 `page must be at least 1`；其他页大小返回 HTTP 400，消息为 `size must be between 1 and 1000`。无法读取的查询字符串（例如 `page=x`）也返回 HTTP 400，正文同样是 JSON 的 `message`。超过最后一页的页码返回空页。
- 行按[处理顺序](/zh-cn/reference/query-language/#处理顺序)排列，最新的在前：先按日期和书写的时刻（在 beancount 账本中，余额断言在当天开始时）；同一时刻内，余额条目（余额断言和所有标记为 `P` 的交易）在其他交易之前，各自按文件中的顺序；`balance ... with pad` 排在同一时刻的其他余额条目（包括它的补齐交易）之后，即张记账检查它的位置。因此余额断言紧挨在它的余额所包含的分录之上。在夏令时跳过某个时刻的那一天，写在跳过区间内的条目位置不变，但显示的是它存储的时刻，即向后推迟跳过区间的长度：纽约 2024-03-10 的 `02:30` 显示为 `03:30`。
- 标记为 `P` 的交易是补齐交易，页面显示为 `BalancePad` 条目。`balance` 行是 `BalanceCheck` 条目，由 `journals.balance_checks` 补全。

#### `journals.postings`

一些交易按书写的分录，按账本顺序排列：数量、数量是否由推算得出、所记入批次的单位成本，以及分录所在账户在该币种下记账前后的余额。

| 参数 | 类型 | 值 |
|------|------|----|
| `ids` | `set` | 交易的 id，即 `journals.page` 一页中的交易 |

```sql
SELECT id, posting_index, account, automatic, balanced,
       first(currency) AS currency,
       sum(number) AS number,
       count(*) AS lots, count(cost_number) AS lots_at_cost,
       min(cost_number) AS cost_number, max(cost_number) AS max_cost_number,
       min(cost_currency) AS cost_currency, max(cost_currency) AS max_cost_currency,
       number(last(only(currency, account_balance))) - sum(number) AS balance_before,
       number(last(only(currency, account_balance))) AS balance_after,
       first(metas) AS metas
WHERE id IN :ids
GROUP BY id, posting_index, account, automatic, balanced
```

- [批次记账](/zh-cn/reference/query-language/#批次记账)拆成多行的分录重新合为一行，数量相加。它的成本是所记入批次的单位成本，因此 10 个单位的 `{{1000 USD}}` 成本为 `100 USD`。减仓记入成本不同的多个批次时没有成本：此时 `cost_number` 与 `max_cost_number`（或两个币种）不同，或 `lots_at_cost` 小于 `lots`。
- 书写时没有金额的分录（`automatic`）在流水中没有数量，只有推算出的数量。
- `balance_before` 和 `balance_after` 是分录所在账户在该币种下记账前后的余额：[`account_balance`](/zh-cn/reference/query-language/#账户余额) 不受 `WHERE` 影响。

#### `journals.balance_checks`

一些余额断言：断言金额及其容差、断言所检查的该账户及其子账户的余额、两者之差，以及断言是否成立。

| 参数 | 类型 | 值 |
|------|------|----|
| `ids` | `set` | 断言的 id，即 `journals.page` 一页中的断言 |

```sql
SELECT id, account, amount, tolerance, actual, amount - actual AS difference, passed
FROM #balances
WHERE id IN :ids
```

本查询与 `accounts.balance_assertions` 用同样的列描述断言，因此流水和账户流水对断言的展示一致。两处 API 响应给断言的字段也相同：`asserted`（即 `amount`）、`checked_balance`（即 `actual` 余额）、`difference`、`tolerance` 和 `passed`。断言不记账，因此在流水中，它前后的余额都是所检查的余额。

#### `journals.payees`

账本中交易的所有收款人，去重并排序，不含补齐交易的收款人。

```sql
SELECT DISTINCT payee
FROM #transactions
WHERE payee IS NOT NULL AND payee != '' AND flag != 'P'
ORDER BY payee
```

#### `journals.accounts`

由 [`account_status`](/zh-cn/reference/query-language/#账户与商品指令) 得出的、在某个日期和时间处于开立状态的账户，按名称排序：即在该时刻写下的交易可以记入的账户。当天以只有日期的 `close` 关闭的账户仍然开立，关闭后重新开立的账户也是开立的。新建交易表单按其交易的日期和时间查询，余额工具按账本时钟查询当前时刻。

| 参数 | 类型 | 值 |
|------|------|----|
| `date` | `date` | 日期，按账本时区 |
| `time` | `str` | 一天中的时间，`HH:MM:SS` |

```sql
SELECT account
FROM #accounts
WHERE account_status(account, :date, :time) = 'open'
ORDER BY account
```

#### `journals.documents`

账本中的所有文档，最新的在前：`document` 指令，以及交易和分录在元数据中写下的文档。

```sql
SELECT date, time, path, account, transaction_id
FROM #documents
ORDER BY seq DESC
```

`path` 是下载文档所用的路径，相对于账本目录。分录写下的文档属于该分录的账户。

#### `journals.errors`

账本错误的一页，先按文件、再按在文件中的位置排列。

| 参数 | 类型 | 值 |
|------|------|----|
| `size` | `int` | 每页的错误数，1 到 1000 |
| `offset` | `int` | 该页之前的错误数：`(page - 1) × size` |

```sql
SELECT id, kind, file, line, column, span_start, span_end, source, metas
FROM #errors
LIMIT :size OFFSET :offset
```

错误列表以文件和指令所在的行表示错误所在的位置（由 `line` 和 `source` 的行数得出）；`span_start` 和 `span_end` 是写入时用来替换该指令的字节偏移。

### 预算与商品

预算页面读取 [`#budgets`](/zh-cn/reference/query-language/#预算表)、[`#budget_definitions`](/zh-cn/reference/query-language/#预算定义表) 和 [`#budget_events`](/zh-cn/reference/query-language/#预算变动表)。月份以其第一天表示，例如 `2024-06-01`；没有指定月份时，页面使用账本时区中的当前月份。

#### `budgets.month`

截至某个月的每个预算：该预算在 `#budgets` 中到这个月为止的最后一个月，即预算页面列出的内容。`#budgets` 中每个预算都有直到当前月份的每一个月，所以 `last_month` 就是所请求的月份，除非请求的是更晚的月份。预算在 `last_month` 之后不可能再有变动，所以 [`CASE`](/zh-cn/reference/query-language/#case) 把它顺延过来：这个月以 `available` 开始，支出为零。`activity` 是一个数值，以预算的 `currency` 计。在该月之后才开始的预算不会列出。`WHERE date <= :month` 还让 `#budgets` 不再生成更晚的月份，所以账本中日期写到遥远未来的笔误不会造成影响。

| 参数 | 类型 | 值 |
|------|------|----|
| `month` | `date` | 该月的第一天 |

```sql
SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency,
       last(date) AS last_month,
       CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned,
       CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity,
       last(available) AS available, last(closed) AS closed
FROM #budgets
WHERE date <= :month
GROUP BY name
ORDER BY name
```

#### `budgets.budget`

单个预算：它的显示名称、分类、商品、其分录计入该预算支出的账户，以及它关闭的日期和时间，来自没有月份的 `#budget_definitions`。没有这个预算时没有结果行。

| 参数 | 类型 | 值 |
|------|------|----|
| `name` | `str` | 预算 |

```sql
SELECT name, alias, category, currency, accounts, close, close_time
FROM #budget_definitions
WHERE name = :name
```

#### `budgets.budget_month`

截至某个月的单个预算，与 `budgets.month` 相同。如果预算在该月之后才开始，则没有结果行，预算页面显示为没有分配、也没有支出。

| 参数 | 类型 | 值 |
|------|------|----|
| `name` | `str` | 预算 |
| `month` | `date` | 该月的第一天 |

```sql
SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency,
       last(date) AS last_month,
       CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned,
       CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity,
       last(available) AS available, last(closed) AS closed
FROM #budgets
WHERE name = :name AND date <= :month
GROUP BY name
```

#### `budgets.events`

某个月中 `budget-add` 和 `budget-transfer` 指令为预算放入的金额，最新的在前，金额按原样给出：转出为负数。

| 参数 | 类型 | 值 |
|------|------|----|
| `name` | `str` | 预算 |
| `month` | `date` | 该月的第一天 |

```sql
SELECT date, time, timestamp, type, amount
FROM #budget_events
WHERE name = :name AND type != 'close' AND yearmonth(date) = :month
ORDER BY timestamp DESC
```

#### `budgets.postings`

某个月中计入预算的分录，最新的在前，每条分录附带其账户在该分录之后、以该分录货币计的余额。它们是 [`budgets`](/zh-cn/reference/query-language/#postings-表) 列含有该预算的分录，所以它们的和就是该月的已支出：其日期和时间当时账户计入该预算（关闭后以其他预算重新开启的账户，从重新开启起计入新的预算）、在预算定义之后到关闭之前、并且有价格能换算为预算货币的分录。预算的 `budget` 指令之前的分录、关闭之后的分录（关闭当天之后，或带时间的 `budget-close` 在该时间之后）以及没有价格可以换算的分录都不列出。预算页面把它们和 `budgets.events` 的事件按时间合并列出，最新的在前。

| 参数 | 类型 | 值 |
|------|------|----|
| `name` | `str` | 预算 |
| `month` | `date` | 该月的第一天 |

```sql
SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units,
       only(currency, account_balance) AS balance
WHERE yearmonth(date) = :month AND :name IN budgets
ORDER BY timestamp DESC
```

商品的精度、前缀、后缀、舍入方式和分组来自它的 `commodity` 指令。账本持有多少该商品、持有在哪些批次中，以及它的价格，来自下面这些查询。持有量指资产（Assets）和负债（Liabilities）账户中的持有量，这些账户用 [`under`](/zh-cn/reference/query-language/#账户函数) 选出，查询因此只读取它们的分录。

#### `commodities.totals`

资产和负债账户持有的每种商品的数量，只列出有持有量的商品。没有结果行的商品持有量为零。

```sql
SELECT currency, sum(number) AS total
WHERE under(account, 'Assets') OR under(account, 'Liabilities')
GROUP BY currency
HAVING sum(number) != 0
ORDER BY currency
```

#### `commodities.total`

资产和负债账户持有的某一种商品的数量。没有持有量时没有结果行。

| 参数 | 类型 | 值 |
|------|------|----|
| `commodity` | `str` | 商品 |

```sql
SELECT currency, sum(number) AS total
WHERE currency = :commodity AND (under(account, 'Assets') OR under(account, 'Liabilities'))
GROUP BY currency
HAVING sum(number) != 0
```

#### `commodities.latest_prices`

每种商品以某种货币计价、截至今日的最新价格，及其日期和时间。汇率与估值所用的一致，来自引擎的价格表：该货币对在今日或之前、任一方向的最新报价，反向报价取其倒数。日期在未来的价格不是最新价格。

| 参数 | 类型 | 值 |
|------|------|----|
| `currency` | `str` | 报价货币，即运营货币 |

```sql
SELECT CASE WHEN currency = :currency THEN currency(amount) ELSE currency END AS currency,
       last(date) AS date, last(time) AS time,
       getprice(last(CASE WHEN currency = :currency THEN currency(amount) ELSE currency END), :currency, today()) AS rate
FROM #prices
WHERE (currency = :currency OR currency(amount) = :currency) AND currency != currency(amount) AND date <= today()
GROUP BY 1
ORDER BY 1
```

#### `commodities.latest_price`

某一种商品以某种货币计价、截至今日的最新价格，及其日期和时间。汇率与估值所用的一致，来自引擎的价格表：该货币对在今日或之前、任一方向的最新报价，反向报价取其倒数。日期在未来的价格不是最新价格。

| 参数 | 类型 | 值 |
|------|------|----|
| `commodity` | `str` | 商品 |
| `currency` | `str` | 报价货币，即运营货币 |

```sql
SELECT CASE WHEN currency = :currency THEN currency(amount) ELSE currency END AS currency,
       last(date) AS date, last(time) AS time, getprice(:commodity, :currency, today()) AS rate
FROM #prices
WHERE ((currency = :commodity AND currency(amount) = :currency) OR (currency = :currency AND currency(amount) = :commodity))
  AND currency != currency(amount) AND date <= today()
GROUP BY 1
```

#### `commodities.lots`

资产和负债账户持有的某种商品的批次：每个账户、每种成本和取得日期的数量，按账户排序，同一账户内最早取得的在前。仅标签不同的批次分开列出。不按成本持有的数量在每个账户中算作一个批次。

| 参数 | 类型 | 值 |
|------|------|----|
| `commodity` | `str` | 商品 |

```sql
SELECT account, cost_date, cost_number, cost_currency, cost_label, sum(number) AS units
WHERE currency = :commodity AND (under(account, 'Assets') OR under(account, 'Liabilities'))
GROUP BY account, cost_date, cost_number, cost_currency, cost_label
HAVING sum(number) != 0
ORDER BY account, cost_date, cost_number, cost_label
```

#### `commodities.prices`

某种商品以任何货币报价的所有价格，最早的在前。

| 参数 | 类型 | 值 |
|------|------|----|
| `commodity` | `str` | 商品 |

```sql
SELECT date, time, amount
FROM #prices
WHERE currency = :commodity
ORDER BY date, time
```
