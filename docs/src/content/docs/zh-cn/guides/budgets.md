---
title: 预算
description: 建立零基预算，为各个类别分配资金，并在网页界面中跟踪支出。
sidebar:
  order: 4
---

张记账的预算是按月划分的信封，采用零基预算的方式：每个月你为预算分配资金，关联到它的支出账户中的支出是它的**已支出**，剩下的是**可用**金额。月末剩余或超支的金额都会结转到下个月。

预算与账户并存：它们从不改变任何余额。完整语法见[预算](/zh-cn/reference/directives/budget/)。

## 创建预算并关联账户

```zhang
2024-01-01 open Expenses:Groceries CNY
  budget: Food
2024-01-01 open Expenses:Restaurants CNY
  budget: Food
2024-01-01 open Expenses:Fun CNY
  budget: Fun

2024-03-01 budget Food CNY
  alias: "餐饮"
  category: "日常"
2024-03-01 budget Fun CNY
  category: "弹性支出"
```

- `budget Food CNY` 创建以 CNY 计的预算 `Food`。`alias` 是网页界面显示的名称，`category` 在预算页面中为预算分组。
- `open` 上的 `budget` 元数据把账户关联到预算。多个账户可以共用一个预算，一个账户也可以重复这个键来指定多个预算。

## 分配和转移资金

```zhang
2024-03-01 budget-add Food 2000 CNY
2024-03-01 budget-add Fun 500 CNY

2024-03-05 * "超市"
  Assets:Bank:Checking -420 CNY
  Expenses:Groceries
2024-03-12 * "火锅"
  Assets:Bank:Checking -1800 CNY
  Expenses:Restaurants

; 火锅太贵了：从 Fun 转一些钱到 Food
2024-03-20 budget-transfer Fun Food 300 CNY

2024-04-01 budget-add Food 2000 CNY
2024-04-03 * "超市"
  Assets:Bank:Checking -380 CNY
  Expenses:Groceries
```

- `budget-add` 在其日期所在的月份为预算分配资金。
- `budget-transfer Fun Food 300 CNY` 在当月把 300 CNY 已分配的资金从 `Fun` 转到 `Food`。

结果如下：

| 月份 | 预算 | 已分配 | 已支出 | 可用 |
|---|---|---|---|---|
| 3 月 | Food | 2,300 | 2,220 | 80 |
| 3 月 | Fun | 200 | 0 | 200 |
| 4 月 | Food | 2,080 | 380 | 1,700 |
| 4 月 | Fun | 200 | 0 | 200 |

4 月，`Food` 以 3 月剩下的 80 CNY 起步，再加上分配的 2,000 CNY。超支的预算以同样的方式结转它的负数金额。

## 支出如何计入

- 关联账户的每条记账行都会计入该预算在交易所在月份的已支出。退款，即负数的记账行，会减少已支出。
- 记账行只从 `budget` 指令的日期起计入。预算存在之前关联账户上的记账行会被跳过，并按账户各报告一次 [`BudgetDoesNotExist`](/zh-cn/reference/error-codes/#budgetdoesnotexist)。
- 金额按原样相加，不做换算。请让预算关联的账户使用预算的货币。

## 关闭预算

```zhang
2024-12-31 budget-close Fun
```

`budget-close` 把预算标记为已关闭，预算页面在每个月都会把它显示为**已关闭**。指定了它的账户仍会把支出计入它，所以也要删除这些账户的 `budget` 元数据。

## 在网页界面中跟踪预算

- **预算**页面一次显示一个月，按类别分组列出每个预算的**已分配**、**已支出**和**可用**金额（没有类别的预算归入**未分类**）。用箭头切换月份。**隐藏未分配金额的预算**会隐藏当月没有分配资金的预算。
- 选择一个预算，可以查看它关联的账户和它当月的活动：分配和转移的资金，以及它的账户上的记账行。
- **总览**页面显示本月的预算以及每个预算已使用的比例。

预算也可以在[查询](/zh-cn/guides/querying/)中使用，即 `#budgets` 表。见查询语言参考中的[预算表](/zh-cn/reference/query-language/#预算表)。

在 Beancount 账本中，预算写作 `custom` 指令。见[从 Beancount 迁移](/zh-cn/getting-started/from-beancount/#兼容性)。
