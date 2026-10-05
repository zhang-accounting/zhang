---
title: 预算
description: budget、budget-add、budget-transfer 和 budget-close 指令的参考，以及如何把账户关联到预算。
sidebar:
  order: 12
---

张记账的预算遵循 YNAB（You Need A Budget）的信封模型：为一个支出类别创建预算，每月给它分配资金，并把支出账户关联到它。这些账户上的支出就是预算的已支出金额，剩下的部分结转到下个月。预算与账户相互独立：它们从不改变账户的余额。

## 语法

```text
YYYY-MM-DD budget <Name> <Commodity>
YYYY-MM-DD budget-add <Name> <Number> <Commodity>
YYYY-MM-DD budget-transfer <FromName> <ToName> <Number> <Commodity>
YYYY-MM-DD budget-close <Name>
```

| 指令 | 作用 |
|---|---|
| `budget` | 创建预算 `<Name>`，以 `<Commodity>` 计。 |
| `budget-add` | 在其日期所在的月份给预算分配一笔金额。负数金额表示收回资金。 |
| `budget-transfer` | 在其日期所在的月份，把已分配的金额从一个预算转到另一个预算。 |
| `budget-close` | 在其日期当天结束时关闭预算；带时间时在该时间关闭。 |

预算名称是一个不含空格、引号、冒号、括号或逗号的单词，例如 `Food` 或 `Daily-Groceries`。每条指令都需要日期，可以附带一天中的时刻，下方可以写元数据行。

`budget` 指令读取两个元数据键：

| 键 | 作用 |
|---|---|
| `alias` | 预算在网页界面中的显示名称。 |
| `category` | 预算页面把这个预算列在哪个分组下。 |

### 关联账户

账户通过其 `open` 指令的 `budget` 元数据计入某个预算。重复这个键可以把账户关联到多个预算。记账行计入其日期当时生效的账户 `open` 所指的预算，所以账户关闭后用其他 `budget` 元数据重新开启，从重新开启起计入新的预算。

```text
YYYY-MM-DD open <Account>
  budget: <Name>
```

## 示例

```zhang
2024-01-01 budget Food CNY
  alias: "Food and groceries"
  category: "Daily"
2024-01-01 budget Fun CNY
  category: "Discretionary"

2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Expenses:Groceries CNY
  budget: Food
2024-01-01 open Expenses:Restaurants CNY
  budget: Food
2024-01-01 open Expenses:Movies CNY
  budget: Fun

2024-01-01 budget-add Food 2000 CNY
2024-01-01 budget-add Fun 300 CNY

2024-01-12 * "Supermarket" "weekly shopping"
  Assets:Bank:Checking -420.00 CNY
  Expenses:Groceries

2024-01-20 budget-transfer Fun Food 100 CNY
```

1 月，`Food` 已分配 2100 CNY，已支出 420 CNY，可用 1680 CNY；`Fun` 可用 200 CNY。

## 行为

预算按月计算。对每个预算和每个月，张记账记录：

- **已分配**（assigned）：`budget-add` 和 `budget-transfer` 放入预算的金额，加上上个月月末的可用金额；
- **已支出**（activity）：当月记到关联账户的记账行。支出账户上的花费使它增加，退款使它减少；
- **可用**（available）：已分配减去已支出。到下个月月初，它成为下个月的已分配金额，所以剩余或超支的金额都会结转。

一些细节：

- 预算从其 `budget` 指令的日期起存在。在这个日期之前记到关联账户的记账行不计入预算，并报告一次 [`BudgetDoesNotExist`](/zh-cn/reference/error-codes/#budgetdoesnotexist)。
- `budget-add` 和 `budget-transfer` 的金额以及记账行，会用账本中在各自日期或之前最新的价格，按各自的日期换算为预算的商品。金额绝不会被当作另一种商品的数字相加：没有价格可以换算的金额不计入，并报告为 [`BudgetCommodityMismatch`](/zh-cn/reference/error-codes/#budgetcommoditymismatch)。
- `budget-close` 关闭预算。只有日期的 `budget-close` 让预算在当天全天仍然有效，带时间的（例如 `2024-03-01 18:00:00 budget-close Food`）在该时间关闭预算。已关闭的预算不再计入支出：关闭之后记到其关联账户的记账行不计入它，每个账户的第一笔会报告为 [`BudgetClosed`](/zh-cn/reference/error-codes/#budgetclosed)。它的 `budget-add` 和 `budget-transfer` 指令仍然计入，所以关闭之后可以用 `budget-transfer` 把剩下的金额转到另一个预算。
- 预算页面从 `budget-close` 所在的月份起把预算显示为已关闭，之前的月份显示为未关闭；总览页面的预算卡片从那个月起不再显示它。之后的 `budget-close` 不会再改变什么。
- 网页界面的预算页面显示每个预算在某个月的已分配、已支出和可用金额，以及某个预算在某个月的事件。在查询中，`#budgets` 和 `#budget_events` 包含同样的数据；见[张记账特有的表](/zh-cn/reference/query-language/#张记账特有的表)。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`DefineDuplicatedBudget`](/zh-cn/reference/error-codes/#defineduplicatedbudget) | `budget` 指令指定的预算已经存在。第二条指令会被忽略。 |
| [`BudgetDoesNotExist`](/zh-cn/reference/error-codes/#budgetdoesnotexist) | `budget-add`、`budget-transfer` 或 `budget-close` 指定的预算在其日期未定义；这条指令会被忽略。或者记账行的账户关联到在记账行日期未定义的预算；这种情况对每个账户和预算报告一次，交易仍会记账。 |
| [`BudgetCommodityMismatch`](/zh-cn/reference/error-codes/#budgetcommoditymismatch) | `budget-add` 或 `budget-transfer` 的金额，或者记到关联账户的记账行，使用的商品与预算不同，并且在其日期或之前没有价格可以换算它。这笔金额不计入预算，交易仍会记账。 |
| [`BudgetClosed`](/zh-cn/reference/error-codes/#budgetclosed) | 记到关联账户的记账行在预算的 `budget-close` 之后。它不计入预算；这种情况对每个账户和预算报告一次，交易仍会记账。 |

## Beancount 兼容性

Beancount 没有预算指令。在 Beancount 文件中，把它们写成 Beancount 本身接受的 `custom` 指令：类型加引号，预算名加引号，`budget` 的货币加引号，`budget-add` 和 `budget-transfer` 的金额直接写：

| 张记账 | Beancount 文件 |
|---|---|
| `2024-01-01 budget Food CNY` | `2024-01-01 custom "budget" "Food" "CNY"` |
| `2024-01-01 budget-add Food 2000 CNY` | `2024-01-01 custom "budget-add" "Food" 2000 CNY` |
| `2024-01-20 budget-transfer Fun Food 100 CNY` | `2024-01-20 custom "budget-transfer" "Fun" "Food" 100 CNY` |
| `2024-12-31 budget-close Food` | `2024-12-31 custom "budget-close" "Food"` |

元数据行照常跟在后面。`bean-check` 和 Fava 把这些行当作普通的 `custom` 条目接受；张记账把它们读作预算，也以这种方式把预算写入 Beancount 文件。早期版本的张记账写出的不加引号的写法 `custom budget Food CNY` 仍然可以读取，现有账本不受影响，但 Beancount 会拒绝它（`Invalid token`）：如果你还用 `bean-check` 检查账本，请改用加引号的写法。带有其他值的 `custom "budget"`，例如 Fava 自己的预算条目（`custom "budget" Expenses:Coffee "daily" 4.00 EUR`），仍然是普通的 [`custom`](/zh-cn/reference/directives/custom/) 指令。

## 相关页面

- [预算](/zh-cn/guides/budgets/)：设置和使用预算。
- [账户](/zh-cn/reference/directives/account/#元数据)：账户的 `budget` 元数据。
