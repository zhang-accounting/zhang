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

账户页面显示该账户**及其子账户**，与账户树一致：它的流水、余额历史、合计和文档都涵盖整棵子树。`Assets:Bank` 的页面包含 `Assets:Bank:Checking` 的分录，流水中的每一行都注明分录所属的账户。余额用 [`convert`](/zh-cn/reference/query-language/#估值函数) 按今天的价格折算为运营货币，它会使用反向价格和持仓的成本货币。

含子账户的余额由这些查询的行相加得到：账户列表和账户页面把一个账户及其下所有账户在 `accounts.balances` 或 `accounts.subtree_balances` 中的行相加，与网页界面的账户树一致。要自己查询一棵子树的合计，可以用 [`under`](/zh-cn/reference/query-language/#账户函数) 筛选：

```sql
SELECT currency, sum(number) AS units
WHERE under(account, 'Assets:Bank')
GROUP BY currency
```

账户页面的流水按 [`seq`](/zh-cn/reference/query-language/#处理顺序)（张记账处理账本的顺序）合并 `accounts.journal` 和 `accounts.balance_assertions` 的行，最新的在前。因此一个断言紧跟在它的余额所包含的分录之后，也就是张记账检查它的位置：写了时刻的余额断言在当天该时刻之前的交易之后，普通的余额断言在写在它之前的补齐之后，`balance ... with pad` 在同一时刻的其他余额记录之后。它的余额就是所在位置的累计余额，它的 `trx_id` 是其检查结果的 id，与 `GET /api/journals` 列出它时使用的 id 相同；分录行的 `trx_id` 是其交易的 id。同一交易的各行按分录最新的在前，按多个批次记账的一笔分录显示为一行。在夏令时跳过某段时间的那天，决定顺序的是写下的时刻，所以写在跳过时段中的行可能排在存储时刻更早的行之前。

页面按每页 100 行列出流水（`GET /api/accounts/{account}/journals?page=1&size=100`），`accounts.journal` 的每一行和每个断言各算一行；一页用 `accounts.journal_rows` 和 `accounts.journal_page` 从流水的末尾读取它的行。

#### `accounts.list`

每个有 `open` 或 `close` 指令的账户，及其开户和销户日期、别名，按名称排序。它和 `accounts.balances` 一起组成账户列表：有分录但没有 `open` 指令的账户也会列出。

```sql
SELECT account, open, close, meta('alias') AS alias
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

```sql
SELECT account, open, close, meta('alias') AS alias
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

该账户及其子账户的分录，按账本顺序排列，每行带有该账户及其子账户在分录货币上紧接其后的[累计余额](/zh-cn/reference/query-language/#累计余额)。`balance ... with pad` 生成的补齐交易与其他交易一样列出。按多个[批次](/zh-cn/reference/query-language/#批次记账)记账的一笔分录每个批次一行，它们的 `seq` 和 `posting_index` 相同。加上 `ORDER BY seq DESC, posting_index DESC` 可以像页面一样把最新的排在最前。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |

```sql
SELECT date, time, timestamp, flag, id, account, payee, narration, seq, posting_index,
       number AS units, currency, only(currency, units(balance)) AS balance
WHERE under(account, :account)
```

#### `accounts.journal_rows`

`accounts.journal` 的行数：加上断言，就是流水所有页的行数。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |

```sql
SELECT count(*) AS rows
WHERE under(account, :account)
```

#### `accounts.journal_page`

`accounts.journal` 的部分行，按账本顺序：流水的一页从末尾数起读取它的行。不论偏移多少，只构建这一页的行。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |
| `limit` | `int` | 行数 |
| `offset` | `int` | 之前跳过的行数 |

```sql
SELECT date, time, timestamp, flag, id, account, payee, narration, seq, posting_index,
       number AS units, currency, only(currency, units(balance)) AS balance
WHERE under(account, :account)
LIMIT :limit OFFSET :offset
```

#### `accounts.balance_assertions`

对该账户的余额断言，按 [`seq`](/zh-cn/reference/query-language/#处理顺序) 最新的在前。`actual` 是断言所检查的该账户及其子账户的余额，也就是流水中它的 `seq` 所在位置的累计余额。`pad` 是 `balance ... with pad` 用来补齐的账户。

| 参数 | 类型 | 值 |
|------|------|----|
| `account` | `str` | 页面的账户 |

```sql
SELECT date, time, timestamp, id, account, amount, actual, passed, pad, seq
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
SELECT date, time, account, path
FROM #documents
WHERE source = 'directive' AND under(account, :account)
```

### 流水

暂无查询。

### 预算与商品

暂无查询。
