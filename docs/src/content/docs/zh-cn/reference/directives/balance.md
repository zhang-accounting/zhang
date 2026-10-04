---
title: 余额
description: "balance 指令的参考：余额断言、显式容差，以及用 balance … with pad 或 pad 指令补齐。"
sidebar:
  order: 5
---

`balance` 指令断言某个账户在某个时间点持有多少某种商品。张记账把断言与该账户记账行的合计进行比较，两者不同时报告错误。加上 `with pad` 时，这条指令还会修正余额：张记账从另一个账户添加一笔补齐交易，使断言成立。与 Beancount 一样的 `pad` 指令，会为其账户之后的断言做同样的事。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] balance <Account> <Number> [~ <Tolerance>] <Commodity>
YYYY-MM-DD [HH:MM[:SS]] balance <Account> <Number> <Commodity> with pad <PadAccount>
YYYY-MM-DD [HH:MM[:SS]] pad <Account> <PadAccount>
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 日期，后面可以跟一天中的时刻（`10:30` 或 `10:30:15`）。见[何时检查](#何时检查)。 |
| `<Account>` | 是 | 要检查的账户。检查涵盖该账户及其所有子账户。 |
| `<Number>` | 是 | 账户应当持有的金额。可以是表达式，例如 `1000 - 35.5`。 |
| `~ <Tolerance>` | 否 | 仍能通过检查的最大差额。没有它时，金额必须精确相等。与 `with pad` 一起使用时会被忽略。 |
| `<Commodity>` | 是 | 要检查的商品。账户中的其他商品不会被检查。 |
| `with pad <PadAccount>` | 否 | 从这个账户补齐差额，通常是一个 `Equity` 账户。 |

指令下方可以写元数据行。`pad` 从 `<PadAccount>` 为 `<Account>` 之后的断言补齐：见 [`pad` 指令](#pad-指令)。

## 示例

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Expenses:Food CNY
2024-01-01 open Equity:Opening-Balances

; 设置期初余额
2024-01-01 balance Assets:Bank:Checking 1000.00 CNY with pad Equity:Opening-Balances

2024-01-05 * "Cafe" "lunch"
  Assets:Bank:Checking -35.50 CNY
  Expenses:Food

; 用银行对账单核对账户
2024-01-31 balance Assets:Bank:Checking 964.50 CNY
```

带容差的断言在余额为 964.49 到 964.51 之间时都会通过：

```zhang
2024-01-31 balance Assets:Bank:Checking 964.50 ~ 0.01 CNY
```

## 行为

### 检查什么

- 余额是断言之前，该账户**及其所有子账户**在被断言商品上所有记账行的合计。`Assets:Bank:Checking` 持有 60 CNY、`Assets:Bank:Savings` 持有 40 CNY 时，`balance Assets:Bank 100 CNY` 通过。被断言的账户本身必须已经开立，父账户也是如此。
- 金额必须精确相等，除非断言用 `~` 给出容差。差额不超过 0.01 时，`964.50 ~ 0.01 CNY` 通过。
- 断言只做检查。不论通过与否，它都不改变任何余额：账户持有的始终是它所有记账行的合计，所有余额、报表和流水显示的都是这个合计。不通过的断言会报告为 [`AccountBalanceCheckError`](/zh-cn/reference/error-codes/#accountbalancecheckerror)。
- 网页界面的流水列出每一条断言，以及断言的金额、检查时所比较的余额和是否通过。在查询中，断言是 [`#balances`](/zh-cn/reference/query-language/#pricesbalancesnoteseventsdocuments-和-commodities) 的行。

### 何时检查

张记账按日期和时间对指令排序。在同一日期和时间内，`open` 和 `commodity` 排在最前，接着是余额断言和补齐交易，然后是按文件顺序排列的其他所有条目。因此：

- 没有时间的断言在当天开始时检查，早于当天的交易。要在这些交易之后检查余额，请把断言的日期写成下一天，或者给它一个时间。
- 有时间的断言，在同一天中时间更早的交易之后检查。没有时间的交易视为午夜。
- 在 Beancount 文件中，张记账与 Beancount 一样忽略 `balance` 的时刻（它的 `time` 元数据）：断言在其日期开始时检查。如果该账户当天在这个时刻之前的交易（早期版本的张记账会把它们计入）改变了它所检查的金额，这条断言会附带一条 [`BalanceTimeIgnored`](/zh-cn/reference/error-codes/#balancetimeignored) 提示。

```zhang
2024-02-01 10:00 * "Cafe" "coffee"
  Assets:Bank:Checking -5.00 CNY
  Expenses:Food

2024-02-01 09:00 balance Assets:Bank:Checking 964.50 CNY
2024-02-01 12:00 balance Assets:Bank:Checking 959.50 CNY
```

### 用 `with pad` 补齐

`balance <Account> <amount> with pad <PadAccount>` 把账户补齐到断言的金额：

1. 张记账计算断言金额与账户在该时刻的余额之差：余额是该账户及其子账户所有记账行的合计，包括在此之前的补齐交易。之前的断言，即使不通过，也不计入。
2. 如果差额不为零，张记账在同一日期和时间添加一笔补齐交易。它的标记是 `P`，收款方是 `Balance Pad`，摘要是 `pad <Account> to <PadAccount>`。它把差额记到 `<Account>` 本身（即使这是一个父账户），并把相反的金额记到 `<PadAccount>`。
3. 已经等于断言金额的账户不会得到补齐交易。

此后，补齐交易就是一笔普通交易：它出现在流水中，改变两个账户的余额，在查询中是一行标记为 `P` 的记录。插件在补齐之前运行，所以补齐金额是在插件添加的所有交易之后计算的。

`balance … with pad` 仍然是一条断言。它在同一日期和时间的所有余额条目都记账之后才检查；当补齐无法让合计达到断言金额时，它会以 [`AccountBalanceCheckError`](/zh-cn/reference/error-codes/#accountbalancecheckerror) 不通过：

- 同一日期和时间里，子账户的补齐在之后改变了合计。请先写子账户的余额，再写父账户的余额。网页界面的批量对账工具就是这样做的。
- 它从被断言的账户本身或其某个子账户补齐：这样的补齐只是在它所断言的合计内部转移数量，永远不会改变合计。Beancount 中这样的补齐同样不通过。

写在 `balance … with pad` 上的 `~ tolerance` 会被忽略：补齐使余额精确相等。

### `pad` 指令

`pad` 指令与 Beancount 一样，从填充账户为其账户之后的余额断言补齐。它在张记账文件和 Beancount 文件中都可以使用：

```zhang
2024-01-01 pad Assets:Bank:Checking Equity:Opening-Balances
2024-02-01 balance Assets:Bank:Checking 1000.00 CNY
```

- 一条 `pad` 为该账户本身在每种商品上、日期晚于这条 `pad` 的第一条 `balance` 服务，直到该账户的下一条 `pad` 为止。比较的是日期而不是时刻，因为 Beancount 没有时刻。
- 与 `pad` 同一天的 `balance` 排在它之前，与 Beancount 对一天的排序相同，不论它们的时刻，都不会被补齐：在带时刻的张记账文件中，`pad` 排在当天最后一条余额断言之后。在 Beancount 文件中，张记账忽略 `pad` 的 `time` 元数据，同一天的 `pad` 保持它们在文件中的行序。
- 它的补齐交易以 `pad` 的日期记账，紧跟在它之后，与 Beancount 相同，所以 `pad` 与断言之间的余额包含这笔补齐。当 `pad` 排在当天更晚的一条余额断言之后时，补齐交易的时刻取那条断言的时刻。补齐多种商品的 `pad` 为每种商品各添加一笔补齐交易。
- 补齐金额在它所服务的断言到来时计算，并计入在它之前已补齐的每条断言的补齐。日期更早、却服务于更晚断言的 `pad` 的补齐不计入：子账户的 `pad` 在父账户的 `pad` 之前，却服务于更晚的余额断言时，父账户的补齐不包含它，父账户的断言会差出这部分补齐而不通过，与 Beancount 相同。
- `pad` 和它所服务的 `balance` 可以在账本的不同文件中。
- 不补齐任何金额（之后没有该账户的断言需要它）的 `pad` 会报告为 [`UnusedPad`](/zh-cn/reference/error-codes/#unusedpad) 错误，与 Beancount 相同。
- `balance … with pad` 补齐它自己的断言，从不报告为未使用。在它之前的 `pad` 优先为它服务。
- 补齐该账户或其某个子账户按成本持有的商品（例如以 `{100 USD}` 买入的股票）会在断言上报告为 [`PadWithCost`](/zh-cn/reference/error-codes/#padwithcost) 错误，与 Beancount 相同。补齐交易不带成本记账。

### 通过网页界面

账户页面上的余额断言表单和批量对账工具，写到 [`directive_output_path`](/zh-cn/reference/directives/options/#directive_output_path) 选项所选的文件中。

- 在张记账文件中，它们写入日期为当前时间的 `balance` 或 `balance … with pad`。
- 在 Beancount 文件中，它们的写法让 Beancount 与张记账的理解一致，并且只在你要求的时候、只补齐你要求的金额：
  - “我现在的余额”写成日期为明天的 `balance`：明天的开始就是今天的结束，排在今天所有交易之后。今天之后再添加的交易不在你断言的金额中，因此会使这条余额断言不通过，与 Beancount 相同。
  - 再次核对会替换文件中明天的那条余额断言，而不是再添加一条。它精确断言新的金额：原有的 `~` 容差会被去掉，它的元数据和注释保持你写的样子。同一条余额断言写了不止一次时，每一处都会修改。网页界面会告诉你替换了哪些余额断言，以及它们原来的金额和容差。
  - 带补齐时，你填写的金额与该账户及其子账户当前持有量之间的差额，会写成一笔日期为当前时间的补齐交易（标记 `P`，收款方 `Balance Pad`），排在 `balance` 之前。没有差额时不写。之前写入的补齐交易会保留，新的补齐从它之后的余额计算差额。不会写 `pad` 指令：它会为账户每种商品的下一条余额断言补齐，并吸收你今天之后添加的交易。
  - 你自己写的 `pad` 仍会为它之后、它从未补齐过的商品的余额断言补齐。这样的余额断言会被拒绝，提示中会写明那条 `pad` 所在的文件和日期：请编辑该文件，在那条 `pad` 之后、它的后一天写一条该商品的余额断言。

在两种文件中，以下请求都会被拒绝，因为写入后只会被报告为错误：

- 已关闭或未开立的账户的余额断言，或从这样的账户补齐；
- 从该账户本身或其子账户补齐：补齐只是在断言的合计内部转移数量，永远不会改变合计；
- 补齐该账户或其子账户在当天按成本持有的商品，这会记入不带成本的数量。请改为以带成本的买入或卖出记录它们。

被拒绝的请求不会写入任何内容，网页界面会显示原因。如果某个需要原地修改的文件在张记账加载之后被编辑过，请求同样不会写入任何内容：它会说明文件已改变，在重新加载的账本上重试即可。当文件无法加载时（例如在文件编辑器中保存了错误的内容），除文件编辑器外的所有写入都会被拒绝，直到你在那里修正它们。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`AccountBalanceCheckError`](/zh-cn/reference/error-codes/#accountbalancecheckerror) | 余额与断言金额之差超过容差；没有容差时，只要有差额。 |
| [`UnusedPad`](/zh-cn/reference/error-codes/#unusedpad) | `pad` 不补齐任何金额：之后没有该账户的断言需要它。 |
| [`PadWithCost`](/zh-cn/reference/error-codes/#padwithcost) | 补齐了该账户或其子账户按成本持有的商品。错误指向断言。 |
| [`BalanceTimeIgnored`](/zh-cn/reference/error-codes/#balancetimeignored) | 提示：在 Beancount 文件中，断言的时刻被忽略，而这改变了它所检查的金额。 |
| [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist) | 账户或补齐账户在该日期未开立。检查和补齐仍会进行。 |
| [`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed) | 账户或补齐账户在该时刻已经关闭。 |
| [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine) | 补齐交易使用了未定义的商品。错误指向这条 `balance … with pad`。 |

## Beancount 兼容性

在 Beancount 文件（`.bean`、`.bc` 或 `.beancount`）中没有 `with pad`：请写 `pad` 指令，张记账按 Beancount 的方式读取它（见 [`pad` 指令](#pad-指令)）。在 Beancount 文件中，一天中的时刻写成 `time: "HH:MM:SS"` 元数据，张记账与 Beancount 一样忽略 `balance` 和 `pad` 上的时刻。

张记账在以下几点与 Beancount 不同：

- 补齐总是使账户精确等于断言金额，即使差额在明确写出的 `~` 容差之内，而 Beancount 此时不补齐。
- 只有对被补齐账户本身的断言会使用这条 `pad`。Beancount 还允许子账户上的断言用掉其父账户的 `pad`，之后父账户自己的断言就得不到补齐。
- 补齐金额计入在它之前已补齐的每条断言的补齐。Beancount 计算父账户的补齐时不计入其子账户的补齐，所以父账户的断言在 Beancount 中会差出这部分补齐而不通过。
- 张记账对同一天的指令先按时刻排序，再按文件（按账本包含文件的顺序），再按行排序。Beancount 只按行号排序，不论文件，也没有时刻。同一账户在不同文件中同一天的两条 `pad`，由张记账排在最后的那条补齐，它可能不是 Beancount 使用的那条。
- 补齐按成本持有的商品时，张记账为它所服务的断言报告一条 `PadWithCost` 错误；Beancount 为每个按成本持有的批次各报告一次。
- Beancount 根据断言金额的小数位数推断容差。张记账不会：没有 `~` 的断言必须精确相等。在原来依赖 Beancount 推断容差的地方，加上 `~ 0.01` 或你需要的容差。

## 相关页面

- [余额](/zh-cn/guides/balances/)：实际运用余额断言和补齐。
- [交易](/zh-cn/reference/directives/transaction/)：补齐交易和其他交易如何记账。
