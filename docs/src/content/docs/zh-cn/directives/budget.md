---
title: 预算指令指南
description: 张记账中使用预算系统的全面指南，包括示例和常见问题解答。
---

## 引言

张记账的预算系统受到了 YNAB (You Need A Budget) 模型的启发，基本上是一种零基预算方法。这个系统引入了几个与预算账户相关的指令和配置，但它们不会影响核心系统的余额计算。例如，将一部分资金分配给预算账户并不会减少任何资产账户的余额。

### 关键指令

#### 创建预算账户

要建立一个预算账户，请使用以下指令：

```zhang
{DATE} budget {BUDGET_NAME} {CURRENCY}
```

##### 元配置选项

- `alias`：（可选）由于 BUDGET_NAME 仅限于英文和下划线，因此 `alias` 提供了更具描述性的显示名称。
- `category`：（可选）有助于在前端进行预算账户的分组展示。

#### 为预算账户增加资金

增加预算金额：

```zhang
{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}
```

#### 转移预算资金

在预算账户之间重新分配资金：

```zhang
{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}
```

示例场景：

```zhang
// 为了在2023年12月节省食物预算给女朋友买礼物，从食物预算中重新分配资金
2023-12-01 budget-transfer Diet GirlFriendGift 200 CNY
```

#### 关闭预算账户

当不再需要预算账户时：

```zhang
{DATE} budget-close {BUDGET_NAME}
```

#### 将支出账户链接到预算

将支出账户链接到预算以跟踪预算使用情况：

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY}
  budget: {BUDGET_NAME}
```

示例：

```zhang
// 将午餐支出账户链接到“饮食”预算
1970-01-01 open Expenses:Lunch CNY
  budget: Diet
```

### Beancount 兼容性

为了确保 Beancount 用户也能使用预算系统，我们对 Beancount 的预算指令进行了兼容和转换，具体语法如下：

- 创建预算账户
    - Zhang: `{DATE} budget {BUDGET_NAME} {CURRENCY}`
    - Beancount: `{DATE} custom budget {BUDGET_NAME} {CURRENCY}`
- 为预算账户增加资金
    - Zhang: `{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - Beancount: `{DATE} custom budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
- 转移预算资金
    - Zhang: `{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - Beancount: `{DATE} custom budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
- 关闭预算账户
    - Zhang: `{DATE} budget-close {BUDGET_NAME}`
    - Beancount: `{DATE} custom budget-close {BUDGET_NAME}`

## 常见问题解答 (FAQ)

### 如何在月中调整预算？

您可以使用 `budget-transfer` 指令在预算账户之间重新分配资金。

### 如果我在一个类别中超支怎么办？

在一个类别中超支不会自动影响您的账户余额。然而，建议调整您的预算以反映实际支出并据此计划。

### 我的预算可以包含多种货币吗？

每个预算账户绑定单一货币。要管理多种货币的预算，请为每种货币创建单独的预算账户。

### 如何关闭预算账户？

使用 `budget-close` 指令并提供您希望关闭的预算账户名称。这将阻止任何进一步的交易被分配到此预算。

有关更详细的示例和高级配置，请参考官方文档和张记账的社区论坛。
