---
title: 内置查询
description: 张记账所显示的各项数字背后有文档说明的 BQL 查询，如何在查询页面打开并修改它们，以及列出这些查询、填入参数值的 HTTP 接口。
---

张记账显示的各项数字正在改为由*内置查询*计算（[#479](https://github.com/zhang-accounting/zhang/issues/479)）。内置查询是用张记账的[查询语言](/zh-cn/user-guide/query-language/)写成的具名查询，页面带着几个参数（例如报表的日期）在你的账本上执行它。页面只负责排列查询结果，所以每个数字背后的逻辑都是一个可以在本页读到的查询。

你可以在**查询**页面打开某个数字背后的查询，页面当时使用的参数值已经填好，然后修改它：换一个日期范围、加入更多账户、换一种分组方式、画成图表。应用能显示的任何内容，你都可以自己查询并加以变化，无需等待新版本。

:::note[迁移进行中]
下面按页面列出内置查询。如果某个页面的小节里还没有查询，说明该页面仍在用代码计算它的数字。
:::

## 打开查询

来自内置查询的数字旁边有 **打开查询** 操作。它会打开查询页面（`/explore`），把查询放进编辑器并执行。参数已经以值的形式写进查询，所以编辑器里的查询是完整的。你可以像对待其他查询一样修改它并再次执行，也可以用 [`query` 指令](/zh-cn/directives/query/)把它保存到账本中。

## 参数

内置查询把它的输入命名为参数，写作 `:name`，例如 `WHERE date >= :from AND date <= :to`。张记账执行查询时绑定参数的值。你在页面中输入的内容（例如搜索关键字）只会作为参数绑定，绝不会被拼接进查询文本。

在查询页面打开查询时，每个参数都会被替换为它的值，写法保证查询能原样读回该值：

| 类型 | 写法 | 示例 |
|------|------|------|
| `date` | `YYYY-MM-DD` | `2024-01-31` |
| `str` | 放在单引号中；如果含有单引号，则放在双引号中 | `'Assets:Bank'`、`"O'Brien"` |
| `set` | 调用 [`set`](/zh-cn/user-guide/query-language/#搜索函数) | `set('food', 'trip')`、`set()` |
| `int` | 数字本身，负数加括号 | `12`、`(-3)` |
| `decimal` | 保留小数位的数字，总带小数点，负数加括号 | `12.50`、`12.`、`(-0.5)` |
| `bool` | `TRUE` 或 `FALSE` | |
| 任意类型 | 没有值时为 `NULL`，通常表示不使用该筛选条件 | `NULL` |

查询语言中的字符串没有转义序列：字符串一直延续到下一个同种引号为止，反斜杠保持原样，所以 `'C:\temp\'` 就是文本 `C:\temp\`。同时含有两种引号的文本会写成字符串的拼接，例如 `("it's " + '"quoted"')`。

写入参数值后的查询返回的行与页面得到的完全相同。只有不带 `AS` 的列名会变化，因为列名就是该列的文本。

## HTTP API

这两个接口与其他 API 一样受[身份认证](/zh-cn/installation/3-authentication/)保护。

### 列出内置查询

`GET /api/query/builtins` 列出所有内置查询，包括名称、说明、BQL 以及参数及其[类型](/zh-cn/user-guide/query-language/#类型)：

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
      ]
    }
  ]
}
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

把这段文本发送到 [`POST /api/query`](/zh-cn/user-guide/query-language/#执行查询) 即可执行。`params` 必须给出查询的每一个参数，且不能有多余的参数，每个值是对应类型的 JSON 值：

| 类型 | JSON 值 |
|------|---------|
| `date` | 字符串 `YYYY-MM-DD` |
| `str` | 字符串 |
| `set` | 字符串列表 |
| `int` | 整数 |
| `decimal` | 数字，或者 `"12.50"` 这样的字符串，以保留小数位 |
| `bool` | `true` 或 `false` |
| 任意类型 | `null` |

查询名称不存在时返回 HTTP 404。参数缺失、多余或类型不对时返回 HTTP 400，`message` 中会指出是哪个参数。

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

### 报表

**报表**页和首页（`GET /api/statistic/summary`、`/api/statistic/graph` 和 `/api/statistic/{account_type}`）。范围是两个账本日期 `from` 和 `to`，含首尾两天；这些接口也接受时间点，表示该时间点在账本时区中所在的那一天。`currency` 是账本的运营货币。

- **估值。**汇总和排行按 `to` 当天的价格估值。图表的每个点按它自己最后一天的价格估值，最后一个点按 `to` 的价格估值。价格可以反向使用；按成本持有、自身没有价格的持仓通过成本货币估值（见 [`convert`](/zh-cn/user-guide/query-language/#估值函数)）。任何价格都换算不了的金额保留原货币，不计入以运营货币表示的合计。
- **图表**每天、每周（周一到周日）或每月一个点，以它的第一天命名，因此第一周（月）和最后一周（月）可能超出范围；报表页面用范围的第一天标注第一个点。范围内没有分录的点沿用前一个点的净资产，最前面的点沿用 `from` 前一天的 `report.net_worth`，并按它自己最后一天的价格估值（见 `report.net_worth_trend`）。
- **限制。**这些数字遵守所有查询的[限制](/zh-cn/user-guide/query-language/#限制)。图表的点数最多为结果大小限制（`ZHANG_QUERY_MAX_RESULT_VALUES`）的一半，默认即 500,000 个；它的点（每种货币一个值）也计入这个限制。按日请求更长的范围会得到 HTTP 400，请改为按周或按月。

#### `report.net_worth`

`to` 当天结束时的净资产（资产与负债的余额），按当天的价格以 `currency` 估值，即汇总中的余额。图表用它求 `from` 前一天的余额，作为起点；`balance` 保留批次，以便在其他日期估值。

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

范围内每个有分录的日、周或月结束时的净资产（资产与负债的余额），按它在范围内最后一天的价格以 `currency` 估值。`interval` 为 `'1 day'`、`'1 week'` 或 `'1 month'`：[`date_bin`](/zh-cn/user-guide/query-language/#日期函数) 从 2001-01-01（既是周一，又是月初）开始划分的区间，就是日历上的日、从周一开始的周和月，每个区间以它的第一天命名。[`least`](/zh-cn/user-guide/query-language/#比较函数) 让最后一个区间的估值日期不超出范围。`balance` 保留批次，用于给没有分录的点估值。

这个查询只列出有分录的日、周或月，因此**打开查询**显示的行比图表的点少。图表把没有分录的日、周或月补上它之前的最后一个余额（来自这个查询，或来自 `from` 前一天的 `report.net_worth`），并按它自己在范围内的最后一天估值，与当天的 `report.net_worth` 的估值相同。查询语言目前还不能列出没有分录的日期。

| 参数 | 类型 | 值 |
|------|------|----|
| `from` | `date` | 第一天 |
| `to` | `date` | 最后一天 |
| `interval` | `str` | `'1 day'`、`'1 week'` 或 `'1 month'` |
| `currency` | `str` | 估值所用的货币 |

```sql
SELECT date_bin(:interval, date, 2001-01-01) AS bucket, last(balance) AS balance, units(last(balance)) AS units,
  convert(last(balance), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY bucket
HAVING max(date) >= :from
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

范围内某一类型账户的十笔最大分录，按 `to` 当天的价格以 `currency` 估值排序，即最大的支出和收入。[`possign`](/zh-cn/user-guide/query-language/#金额与数值) 把收入和负债变为正数，因此最大的收入排在最前。任何价格都无法换算为 `currency` 的分录排在最后。`account_balance` 是该分录之后其账户以分录货币计的余额。

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

暂无查询。

### 流水

暂无查询。

### 预算与商品

暂无查询。
