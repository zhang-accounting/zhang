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

暂无查询。

### 账户

暂无查询。

### 流水

暂无查询。

### 预算与商品

预算页面读取 [`#budgets`](/zh-cn/user-guide/query-language/#预算表)、[`#budget_definitions`](/zh-cn/user-guide/query-language/#预算定义表) 和 [`#budget_events`](/zh-cn/user-guide/query-language/#预算变动表)。月份以其第一天表示，例如 `2024-06-01`；没有指定月份时，页面使用账本时区中的当前月份。

#### `budgets.month`

截至某个月的每个预算：该预算在 `#budgets` 中到这个月为止的最后一个月，即预算页面列出的内容。`#budgets` 中每个预算都有直到当前月份的每一个月，所以 `last_month` 就是所请求的月份，除非请求的是更晚的月份。预算在 `last_month` 之后不可能再有变动，所以 [`CASE`](/zh-cn/user-guide/query-language/#case) 把它顺延过来：这个月以 `available` 开始，支出为零。在该月之后才开始的预算不会列出。`WHERE date <= :month` 还让 `#budgets` 不再生成更晚的月份，所以账本中日期写到遥远未来的笔误不会造成影响。

| 参数 | 类型 | 值 |
|------|------|----|
| `month` | `date` | 该月的第一天 |

```sql
SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency,
       last(date) AS last_month,
       CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned,
       CASE WHEN last(date) < :month THEN last(available) * 0 ELSE last(activity) END AS activity,
       last(available) AS available, last(closed) AS closed
FROM #budgets
WHERE date <= :month
GROUP BY name
ORDER BY name
```

#### `budgets.budget`

单个预算：它的显示名称、分类、商品，以及其分录计入该预算支出的账户，来自没有月份的 `#budget_definitions`。没有这个预算时没有结果行。

| 参数 | 类型 | 值 |
|------|------|----|
| `name` | `str` | 预算 |

```sql
SELECT name, alias, category, currency, accounts
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
       CASE WHEN last(date) < :month THEN last(available) * 0 ELSE last(activity) END AS activity,
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

某个月中预算账户的分录，最新的在前，每条分录附带其账户在该分录之后、以该分录货币计的余额。预算页面把它们和 `budgets.events` 的事件按时间合并列出，最新的在前。

| 参数 | 类型 | 值 |
|------|------|----|
| `accounts` | `set` | 预算的账户，即 `budgets.budget` 的 `accounts` |
| `month` | `date` | 该月的第一天 |

```sql
SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units,
       only(currency, account_balance) AS balance
WHERE account IN :accounts AND yearmonth(date) = :month
ORDER BY timestamp DESC
```

商品的精度、前缀、后缀、舍入方式和分组来自它的 `commodity` 指令。账本持有多少该商品、持有在哪些批次中，以及它的价格，来自下面这些查询。持有量指资产（Assets）和负债（Liabilities）账户中的持有量，这些账户用 [`under`](/zh-cn/user-guide/query-language/#账户函数) 选出，查询因此只读取它们的分录。

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

每种商品以某种货币报价的最新价格，及其日期和时间。

| 参数 | 类型 | 值 |
|------|------|----|
| `currency` | `str` | 报价货币，即运营货币 |

```sql
SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price
FROM #prices
WHERE currency(amount) = :currency
GROUP BY currency
ORDER BY currency
```

#### `commodities.latest_price`

某一种商品以某种货币报价的最新价格，及其日期和时间。

| 参数 | 类型 | 值 |
|------|------|----|
| `commodity` | `str` | 商品 |
| `currency` | `str` | 报价货币，即运营货币 |

```sql
SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price
FROM #prices
WHERE currency = :commodity AND currency(amount) = :currency
GROUP BY currency
```

#### `commodities.lots`

资产和负债账户持有的某种商品的批次：每个账户、每种成本和取得日期的数量，按账户排序，同一账户内最早取得的在前。不按成本持有的数量在每个账户中算作一个批次。

| 参数 | 类型 | 值 |
|------|------|----|
| `commodity` | `str` | 商品 |

```sql
SELECT account, cost_date, cost_number, cost_currency, sum(number) AS units
WHERE currency = :commodity AND (under(account, 'Assets') OR under(account, 'Liabilities'))
GROUP BY account, cost_date, cost_number, cost_currency
HAVING sum(number) != 0
ORDER BY account, cost_date, cost_number
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
