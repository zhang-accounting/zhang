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
- 其他货币的记账行会用账本中的价格，按其日期换算为预算的货币。没有价格可以换算的记账行不计入，绝不会被当作另一种货币的数字相加，并报告为 [`BudgetCommodityMismatch`](/zh-cn/reference/error-codes/#budgetcommoditymismatch)：添加 `price` 即可计入。其他货币的 `budget-add` 或 `budget-transfer` 也是如此。
- 记账行计入其日期和时间当时账户 `open` 所指的预算。如果关闭一个账户后用其他 `budget` 元数据重新开启，之后的记账行计入新的预算，之前的记账行仍属原来的预算，即使账户是在同一天稍后重新开启。

## 关闭预算

```zhang
2024-12-31 budget-close Fun
```

`budget-close` 关闭预算：预算页面从其日期所在的月份起把它显示为**已关闭**，之前的月份显示为未关闭。第二条 `budget-close` 不会再改变什么。

已关闭的预算不再计入支出。预算在 `budget-close` 当天全天仍然有效；带时间时（`2024-12-31 18:00:00 budget-close Fun`）有效到该时间。之后记到其账户的记账行不计入它，张记账会把每个账户的第一笔报告为 [`BudgetClosed`](/zh-cn/reference/error-codes/#budgetclosed)：请删除这些账户的 `budget` 元数据，或者把它们关联到另一个预算。关闭之后 `budget-add` 和 `budget-transfer` 仍然计入，所以可以把剩下的金额转到另一个预算：

```zhang
2025-01-02 budget-transfer Fun Food 200 CNY
```

## 在网页界面中跟踪预算

- **预算**页面一次显示一个月，按类别分组列出每个预算的**已分配**、**已支出**和**可用**金额（没有类别的预算归入**未分类**）。用箭头切换月份。**隐藏未分配金额的预算**会隐藏当月没有分配资金的预算。页面顶部和各类别的当月合计累加当月所有开放的预算，无论是否隐藏：在该月或之前关闭的预算仍会列出，但不再计入合计。
- 选择一个预算，可以查看它关联的账户和它当月的活动：分配和转移的资金，以及计入它的记账行。列出的记账行之和就是当月的**已支出**：预算存在之前、关闭之后或没有价格可以换算的记账行不会列出。
- **总览**页面显示本月的预算以及每个预算已使用的比例。其中的剩余金额与**预算**页面按同样的方式累加。

预算也可以在[查询](/zh-cn/guides/querying/)中使用，即 `#budgets` 表。见查询语言参考中的[预算表](/zh-cn/reference/query-language/#预算表)。

在 Beancount 账本中，预算写作 `custom` 指令。见[从 Beancount 迁移](/zh-cn/getting-started/from-beancount/#兼容性)。
