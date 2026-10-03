---
title: 内置查询
description: 张记账网页界面各页面背后的具名查询及其 BQL：应用中显示的每个数字，都可以在探索页面查询，并按需修改。
---

网页界面的各个页面用张记账[查询语言](/zh-cn/user-guide/query-language/)中的具名查询计算它们显示的数字。本页列出这些查询及其 BQL：应用中显示的任何内容，都可以在探索页面查询，也可以修改查询来回答相关的问题。

- 查询以参数的形式读取运行它的页面的值：`:account` 是账户页面的账户，`:operating_currency` 是账本的 `operating_currency` 选项。自己运行查询时，把每个参数换成一个值，例如 `'Assets:Bank'` 或 `'CNY'`。
- `today()` 是账本时区中的当前日期。

## 账户

账户页面显示该账户**及其子账户**，与账户树一致：它的流水、余额历史、合计和文档都涵盖整棵子树。`Assets:Bank` 的页面包含 `Assets:Bank:Checking` 的分录，流水中的每一行都注明分录所属的账户。余额用 [`convert`](/zh-cn/user-guide/query-language/#估值函数) 按今天的价格折算为运营货币，它会使用反向价格和持仓的成本货币。

### accounts

每个有 `open` 或 `close` 指令的账户，及其开户和销户日期、别名，按名称排序。它和 `account_balances` 一起组成账户列表：有分录但没有 `open` 指令的账户也会列出。

```sql
SELECT account, open, close, meta('alias') AS alias
FROM #accounts
ORDER BY account
```

### account_balances

每个有分录的账户自身分录的余额，按货币分行：数量、按今天的价格折算为运营货币的价值，以及第一笔分录的日期。账户列表把一个账户及其下所有账户的行相加，得到含子账户的余额。

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
GROUP BY account, currency
ORDER BY account, currency
```

### account_subtree

一个账户及其有 `open` 或 `close` 指令的子账户。账户页面从中读取日期、状态和别名。

```sql
SELECT account, open, close, meta('alias') AS alias
FROM #accounts
WHERE under(account, :account)
ORDER BY account
```

### account_subtree_balances

一个账户及其每个子账户的余额，与 `account_balances` 相同。账户页面显示这些行的合计，也就是对该账户的 `balance` 断言所检查的余额，以及该账户自身的余额。

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
WHERE under(account, :account)
GROUP BY account, currency
ORDER BY account, currency
```

### account_journal

账户页面的流水：该账户及其子账户的分录，最新的在前，每笔分录一行，每行带有该账户及其子账户在分录货币上紧接其后的[累计余额](/zh-cn/user-guide/query-language/#累计余额)。`balance ... with pad` 生成的补齐交易与其他交易一样列出。

```sql
SELECT date, time, timestamp, flag, id, account, payee, narration, currency,
       sum(number) AS units,
       last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index, date, time, timestamp, flag, id, account, payee, narration, currency
ORDER BY seq DESC, posting_index DESC
```

### account_balance_assertions

对该账户的余额断言，列在它的流水中。`actual` 是断言所检查的该账户及其子账户的余额：流水把每个断言放在累计余额等于这个余额的位置，即当天开始时、它所包含的补齐之后。

```sql
SELECT date, time, timestamp, id, account, amount, actual, passed, pad
FROM #balances
WHERE account = :account
ORDER BY seq DESC
```

### account_balance_history

账户页面的余额历史图：该账户及其子账户在每个有分录的日子结束时的余额，按货币分开，按日期排序。

```sql
SELECT date, currency, last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY date, currency
ORDER BY date, currency
```

### account_documents

账户页面的文档：该账户及其子账户的 `document` 指令，按账本顺序排列。`path` 是文件相对于账本目录的路径，页面用它下载文件。

```sql
SELECT date, time, account, path
FROM #documents
WHERE source = 'directive' AND under(account, :account)
```
