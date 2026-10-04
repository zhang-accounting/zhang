---
title: 内置查询
description: 张记账所显示的各项数字背后有文档说明的 BQL 查询，如何在查询页面打开并修改它们，以及列出这些查询、填入参数值的 HTTP 接口。
---

张记账显示的各项数字正在改为由*内置查询*计算（[#479](https://github.com/zhang-accounting/zhang/issues/479)）。内置查询是用张记账的[查询语言](/zh-cn/reference/query-language/)写成的具名查询，页面带着几个参数（例如报表的日期）在你的账本上执行它。页面只负责排列查询结果，所以每个数字背后的逻辑都是一个可以在本页读到的查询。

你可以在**查询**页面打开某个数字背后的查询，页面当时使用的参数值已经填好，然后修改它：换一个日期范围、加入更多账户、换一种分组方式、画成图表。应用能显示的任何内容，你都可以自己查询并加以变化，无需等待新版本。

:::note[迁移进行中]
下面按页面列出内置查询。如果某个页面的小节里还没有查询，说明该页面仍在用代码计算它的数字。
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

这两个接口与其他 API 一样受[身份认证](/zh-cn/deployment/authentication/)保护。

### 列出内置查询

`GET /api/query/builtins` 列出所有内置查询，包括名称、说明、BQL 以及参数及其[类型](/zh-cn/reference/query-language/#类型)：

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

把这段文本发送到 [`POST /api/query`](/zh-cn/reference/query-language/#执行查询) 即可执行。`params` 必须给出查询的每一个参数，且不能有多余的参数，每个值是对应类型的 JSON 值：

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

- 页数按 `LIMIT` 和 `OFFSET` 之前的总行数计算。`GET /api/journals` 和 `GET /api/errors` 的页大小 `size` 为 1 到 1000，默认 100；其他大小返回 HTTP 400，消息为 `size must be between 1 and 1000`；超过最后一页的页码返回空页。
- 行按[处理顺序](/zh-cn/reference/query-language/#处理顺序)排列，最新的在前：先按日期和书写的时刻；同一时刻内，余额条目（余额断言和所有标记为 `P` 的交易）在其他交易之前，各自按文件中的顺序；`balance ... with pad` 排在同一时刻的其他余额条目（包括它的补齐交易）之后，即张记账检查它的位置。因此余额断言紧挨在它的余额所包含的分录之上。在夏令时跳过某个时刻的那一天，写在跳过区间内的条目位置不变，但显示的是它存储的时刻，即跳过区间之后的第一个时刻。
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

一些余额断言：断言金额、账户的真实余额、两者之差，以及断言是否成立。

| 参数 | 类型 | 值 |
|------|------|----|
| `ids` | `set` | 断言的 id，即 `journals.page` 一页中的断言 |

```sql
SELECT id, account, amount, tolerance, actual, passed,
       amount - actual AS difference,
       actual + (amount - actual) AS asserted
FROM #balances
WHERE id IN :ids
```

`asserted` 是断言金额，按余额的小数位数书写。

#### `journals.payees`

账本中交易的所有收款人，去重并排序，不含补齐交易的收款人。

```sql
SELECT DISTINCT payee
FROM #transactions
WHERE payee IS NOT NULL AND payee != '' AND flag != 'P'
ORDER BY payee
```

#### `journals.accounts`

未关闭的账户，按名称排序。

```sql
SELECT account
FROM #accounts
WHERE open IS NOT NULL AND close IS NULL
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
SELECT id, kind, file, span_start, span_end, source, metas
FROM #errors
LIMIT :size OFFSET :offset
```

错误列表以文件和指令在其中的字节偏移表示错误所在的位置。

### 预算与商品

暂无查询。
