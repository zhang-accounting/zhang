---
title: 查询
description: 使用查询页面回答关于账本的问题，并从常用查询入手。
sidebar:
  order: 6
---

三月份我们在吃的方面花了多少？季度末银行里有多少钱？住酒店的钱花到哪里去了？**查询**页面用一门与 Beancount 查询语言（BQL）兼容的查询语言回答这类问题。本指南介绍如何使用这个页面，并给出一些可以作为起点的查询。每个子句、表和函数都在[查询语言](/zh-cn/reference/query-language/)中有说明。

## 查询页面

从侧边栏的**更多**分组中打开**查询**。

- 在编辑器中输入查询并选择**运行**，或按 <kbd>Ctrl</kbd>+<kbd>Enter</kbd>（macOS 上为 <kbd>Cmd</kbd>+<kbd>Enter</kbd>）。
- 结果是一张表。由一个标签列和一个金额列组成的结果还会绘制成图表：随时间变化的折线图、柱状图，或者在标签是账户时绘制成矩形树图。用**表格**和**图表**开关选择要看的视图。
- **示例**把一条现成的查询放进编辑器并运行。
- **已保存**列出保存在账本中的查询（见[下文](#保存查询)）。
- **参考**列出表、列和函数。选择其中一项即可把它插入编辑器。
- 如果查询有错误，编辑器会指出出错的行和列。

查询是只读的：它们从不修改你的账本。

## 常用查询

下面的结果来自这个小账本：

<details>
<summary>示例账本</summary>

```zhang title="main.zhang"
option "title" "Household"
option "operating_currency" "CNY"

1970-01-01 commodity CNY

2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Assets:Cash CNY
2024-01-01 open Liabilities:CreditCard CNY
2024-01-01 open Income:Salary CNY
2024-01-01 open Expenses:Food:Groceries CNY
2024-01-01 open Expenses:Food:Restaurants CNY
2024-01-01 open Expenses:Transport CNY
2024-01-01 open Expenses:Travel CNY
2024-01-01 open Equity:Opening-Balances CNY

2024-01-01 balance Assets:Bank:Checking 5000 CNY with pad Equity:Opening-Balances
2024-01-01 balance Assets:Cash 300 CNY with pad Equity:Opening-Balances

2024-03-01 * "ACME Corp" "March salary"
  Assets:Bank:Checking 12000 CNY
  Income:Salary
2024-03-02 * "Supermarket" "Weekly shopping"
  Liabilities:CreditCard -386.50 CNY
  Expenses:Food:Groceries
2024-03-05 * "Noodle House" "Lunch" #work
  Liabilities:CreditCard -48 CNY
  Expenses:Food:Restaurants
2024-03-09 * "Hotel" "Two nights in Hangzhou" #trip-hangzhou
  Liabilities:CreditCard -840 CNY
  Expenses:Travel
2024-03-10 * "Metro" "Top-up"
  Assets:Cash -100 CNY
  Expenses:Transport
2024-03-16 * "Supermarket" "Weekly shopping"
  Liabilities:CreditCard -412.30 CNY
  Expenses:Food:Groceries
2024-03-25 * "Bank" "Credit card bill"
  Assets:Bank:Checking -1686.80 CNY
  Liabilities:CreditCard
2024-04-01 * "ACME Corp" "April salary"
  Assets:Bank:Checking 12000 CNY
  Income:Salary
2024-04-06 * "Supermarket" "Weekly shopping"
  Liabilities:CreditCard -295.00 CNY
  Expenses:Food:Groceries
```

</details>

每个查询读取的都是**记账行**：每条记账行一行，包含其所属交易的日期、收款方和摘要，以及它的 `account` 和 `position`（金额）。`~` 在文本的任意位置匹配正则表达式，并且不区分大小写，`^` 把匹配锚定在开头。

### 某月各类别的支出

```sql
SELECT account, sum(position) AS spent
WHERE account ~ '^Expenses' AND year = 2024 AND month = 3
GROUP BY account
ORDER BY account
```

| account | spent |
|---|---|
| Expenses:Food:Groceries | 798.80 CNY |
| Expenses:Food:Restaurants | 48 CNY |
| Expenses:Transport | 100 CNY |
| Expenses:Travel | 840 CNY |

标签都是账户，所以页面还会把这个结果绘制成矩形树图。

### 每月支出

```sql
SELECT yearmonth(date) AS month, sum(position) AS spent
WHERE account ~ '^Expenses'
GROUP BY month
ORDER BY month
```

| month | spent |
|---|---|
| 2024-03-01 | 1786.80 CNY |
| 2024-04-01 | 295.00 CNY |

`yearmonth(date)` 是记账行所在月份的第一天。以日期为标签时，图表是随时间变化的折线图。

### 账户在某日的余额

```sql
SELECT account, sum(position) AS balance
WHERE account ~ '^Assets:Bank:Checking' AND date < 2024-04-01
GROUP BY account
```

| account | balance |
|---|---|
| Assets:Bank:Checking | 15313.20 CNY |

这个查询把 2024 年 4 月 1 日之前的所有记账行相加，所以得到的是 3 月底的余额。要一次得到多个账户的余额，可以用带 `CLOSE ON` 的 `BALANCES`，它给出某个日期的资产负债表：

```sql
BALANCES FROM CLOSE ON 2024-04-01 WHERE account ~ '^(Assets|Liabilities)'
```

| account | sum(position) |
|---|---|
| Assets:Bank:Checking | 15313.20 CNY |
| Assets:Cash | 200 CNY |
| Liabilities:CreditCard | |

记账行合计为零的账户，例如还清账单后的信用卡，显示为空金额。

### 在某个收款方的全部消费

```sql
SELECT date, narration, account, position
WHERE payee = 'Supermarket' AND account ~ '^Expenses'
ORDER BY date
```

| date | narration | account | position |
|---|---|---|---|
| 2024-03-02 | Weekly shopping | Expenses:Food:Groceries | 386.50 CNY |
| 2024-03-16 | Weekly shopping | Expenses:Food:Groceries | 412.30 CNY |
| 2024-04-06 | Weekly shopping | Expenses:Food:Groceries | 295.00 CNY |

如果要按收款方汇总，就按收款方分组：`SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee`。

### 带累计余额的账户流水

```sql
JOURNAL 'Liabilities:CreditCard' FROM year = 2024 AND month = 3
```

`JOURNAL` 列出匹配该模式的账户的记账行，最后一列是累计余额：

| date | payee | narration | position | balance |
|---|---|---|---|---|
| 2024-03-02 | Supermarket | Weekly shopping | -386.50 CNY | -386.50 CNY |
| 2024-03-05 | Noodle House | Lunch | -48 CNY | -434.50 CNY |
| 2024-03-09 | Hotel | Two nights in Hangzhou | -840 CNY | -1274.50 CNY |
| 2024-03-16 | Supermarket | Weekly shopping | -412.30 CNY | -1686.80 CNY |
| 2024-03-25 | Bank | Credit card bill | 1686.80 CNY | |

这张表省略了结果中的 `flag` 和 `account` 列，并简写了收款方和摘要两列的列名，它们实际是 `maxwidth(payee, 48)` 和 `maxwidth(narration, 80)`。

### 带标签的记账行

```sql
SELECT date, payee, account, position
WHERE 'trip-hangzhou' IN tags
```

| date | payee | account | position |
|---|---|---|---|
| 2024-03-09 | Hotel | Liabilities:CreditCard | -840 CNY |
| 2024-03-09 | Hotel | Expenses:Travel | 840 CNY |

### 大额支出

```sql
SELECT date, payee, account, position
WHERE account ~ '^Expenses' AND number > 400
ORDER BY date
```

| date | payee | account | position |
|---|---|---|---|
| 2024-03-09 | Hotel | Expenses:Travel | 840 CNY |
| 2024-03-16 | Supermarket | Expenses:Food:Groceries | 412.30 CNY |

`number` 是记账行金额的数值部分，不含商品。

### 更多

- [余额](/zh-cn/guides/balances/#断言失败时)用 `#balances` 表查找失败的余额断言。
- [批次与成本](/zh-cn/guides/lots-and-cost-basis/#查看批次)按成本和市场价格为持仓估值。
- 参考文档中的[示例](/zh-cn/reference/query-language/#示例)包括损益表、资产负债表和投资组合估值。

## 保存查询

用 [`query`](/zh-cn/reference/directives/query/) 指令把常用的查询保存在账本中：

```zhang
2024-03-31 query "food by payee" "SELECT payee, sum(position) AS spent WHERE account ~ '^Expenses:Food' GROUP BY payee ORDER BY payee"
```

它会以自己的名字出现在查询页面的**已保存**菜单中。选择它会把查询放进编辑器并运行。

## 导出结果

**导出 CSV** 把编辑器中查询的结果下载为 `query.csv`。每个金额列会按商品拆成各自的数值列，因此文件可以直接在电子表格中打开：

```text
account,spent (CNY)
Expenses:Food:Groceries,798.80
Expenses:Food:Restaurants,48
Expenses:Transport,100
Expenses:Travel,840
```

脚本也可以通过 HTTP 运行查询，使用 `POST /api/query` 和 `POST /api/query/csv`。见 [HTTP API](/zh-cn/reference/query-language/#http-api)。
