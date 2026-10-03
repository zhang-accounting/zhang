---
title: 错误码
description: 张记账为账本报告的每个错误码，触发它的原因以及修正方法。
sidebar:
  order: 15
---

张记账加载账本时，会把发现的问题报告为错误。账本仍会加载：网页界面的错误页面列出这些错误，以及引发每个错误的指令；`GET /api/errors` 和 [`#errors`](/zh-cn/reference/query-language/#错误表) 查询表也是如此。每个错误都有一个错误码，下面列出每个错误码，以及错误页面在中文界面中为它显示的提示。

有些问题会让账本无法加载，它们没有错误码。见[账本无法加载的情况](#账本无法加载的情况)。

## UnbalancedTransaction

*交易不平衡*

交易的记账行在某种商品上加起来不为零。与 Beancount 一样，每种商品都必须各自配平。记账行贡献它的*权重*：

- 普通记账行：它的数量；
- `10 USD @ 7 CNY`（70 CNY）或 `10 USD @@ 70 CNY`：按价格换算的数量；
- 带成本的记账行：按成本计算的数量。对于 `{}`，是该记账行所减批次的成本，所以面对批次 `10 USD {10 CNY}` 和 `10 USD {11 CNY}`（FIFO），`-15 USD {}` 的权重为 `-155 CNY`。

每种商品的合计先按该商品的 `precision` 和 `rounding` 舍入，再与零比较，所以精度为 2 时，`0.004` 的差额可以通过。如果权重所用的商品未定义，则改为报告 [`CommodityDoesNotDefine`](#commoditydoesnotdefine)。

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 CNY
  Expenses:Food 12 CNY
```

交易仍按写下的内容记账。**修正方法**：改正金额，或者省略一个记账行的金额，让张记账推断它。见[交易如何配平](/zh-cn/reference/directives/transaction/#交易如何配平)。

## TransactionCannotInferTradeAmount

*无法推断交易金额*

一个记账行省略了金额，但没有可以推断它的依据：其他记账行没有金额，或者它们在多种商品上配平。

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Cash
```

如果其他记账行已经在单一商品上配平，省略金额的记账行得到该商品的零（带隐含收益的按成本卖出会记为 `0 CNY`），所以流水仍显示你写下的记账行；Beancount 则会丢弃这样的记账行。

推断出的金额是精确的：金额、成本、价格及其乘积从不舍入。只有张记账需要做除法的成本，例如分摊到 3 个单位上的总成本 `{{1000 USD}}`（每单位 333.333… USD），才会产生超过 20 位的小数；这样的金额按该商品的 `rounding` 舍入，舍入到该商品的 `precision` 与交易中该商品写出的最多小数位数两者中较大的那个。之后卖出全部 3 个单位，正好得到 `1000 USD`。

交易**不记账**。**修正方法**：写出其他记账行的金额，或者这个记账行的金额。

## TransactionHasMultipleImplicitPosting

*该交易存在多条隐形/需推导金额的行*

不止一个记账行省略了金额。与 Beancount 一样，每笔交易只能有一个记账行这样做。

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Assets:Card
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Expenses:Food 10 CNY
  Assets:Cash
  Assets:Card
```

交易**不记账**。**修正方法**：除一个记账行外，写出所有记账行的金额。

## TransactionExplicitPostingHaveMultipleCommodity

*交易的显式记录使用了多种货币*

一个记账行省略了金额，而其他记账行的权重在不止一种商品上不平衡，所以张记账无法判断缺失的金额属于哪种商品。

```zhang
2024-01-01 commodity USD
2024-01-01 open Assets:Cash
2024-01-01 open Assets:Card
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Expenses:Food 10 CNY
  Assets:Card -5 USD
  Assets:Cash
```

省略金额的记账行，是在其他记账行匹配批次之后，根据它们的权重推断的。减掉的数量多于批次持有量的 `{}` 卖出，会留下一部分没有成本，这部分按其数量计算权重：这就是第二种商品，所以这笔交易在 [`NoEnoughCommodityLot`](#noenoughcommoditylot) 之后还会得到这个错误。

交易**不记账**。**修正方法**：写出每个金额，或者用价格（`@`）换算，让权重都使用同一种商品。

## AccountBalanceCheckError

*账户定期对账不通过*

[`balance`](/zh-cn/reference/directives/balance/) 断言不成立：账户及其子账户持有的该商品数量不同，差额超过断言的 `~` 容差；没有容差时，只要有差额就不成立。`balance … with pad` 在补齐无法让合计达到断言金额时也会不成立。

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Equity:Opening-Balances

2024-01-01 * "Opening balance"
  Assets:Cash 100 CNY
  Equity:Opening-Balances

2024-01-02 balance Assets:Cash 500 CNY
```

错误的 `account_name` 元数据指出被断言的账户。不成立的断言只报告这个错误：它不改变任何余额，账户在各处都保持其记账行的合计。流水显示这条断言，以及断言的金额和实际余额。

**修正方法**：找到缺失或错误的交易并改正它。注意，没有时间的断言在当天的交易之前检查。要有意设置余额，请使用 [`balance … with pad`](/zh-cn/reference/directives/balance/#用-with-pad-补齐)。

## AccountDoesNotExist

*对应账户不存在*

指令使用了一个在它之前没有 `open` 的账户：交易的记账行、`balance`（它的账户或补齐账户）、`note`、`document` 或 `close`。这个账户从未开立、开立日期更晚，或者名称有拼写错误。

```zhang
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Csh -10 CNY
  Expenses:Food 10 CNY
```

每个账户在每条指令上报告一次，错误的 `account_name` 元数据给出该账户。指令仍然生效：交易照常记账，断言照常检查。**修正方法**：改正账户名，或者在首次使用之日或之前添加一条 `open`。

## AccountClosed

*尝试使用一个已经关闭的账户*

指令在账户的 `close` 之后使用它：日期在 `close` 当天之后的交易、在它之后的 `balance` 或 `document`，或者第二条 `close`。与 Beancount 一样，账户在 `close` 当天全天仍可使用，`close` 之后的 `note` 也不会报错。

```zhang
2024-01-01 open Assets:Old-Card
2024-01-01 open Expenses:Food
2024-03-31 close Assets:Old-Card

2024-04-02 * "Cafe" "lunch"
  Assets:Old-Card -10 CNY
  Expenses:Food 10 CNY
```

每个账户在每笔交易上报告一次，交易仍会记账。**修正方法**：记到另一个账户、改正日期，或者用新的 `open` 重新开立账户。

## CommodityDoesNotDefine

*尝试使用一个未定义的货币*

在 `commodity` 指令定义某种商品之前就使用了它：

- 交易以它配平，作为数量、价格或成本（错误带有交易的 `txn_id` 元数据）；
- `open` 列出了它，或者 `price` 提到了它（错误带有 `commodity_name` 元数据）；
- `balance … with pad` 的补齐交易使用它。

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 USD
  Expenses:Food 10 USD
```

[主货币](/zh-cn/reference/directives/options/#operating_currency)由它的选项定义，所以没有这个选项的账本可以不写 `commodity` 指令就使用 `CNY`。**修正方法**：在首次使用之日或之前添加一条 `commodity` 指令。同一日期内，把它写在列出它的 `open` 上方。

## NoEnoughCommodityLot

*没有足够的货币批次用于记账*

带成本的记账行所减的数量，多于账户中匹配批次持有的数量。

```zhang
2024-01-01 commodity USD
2024-01-01 commodity AAPL
2024-01-01 open Assets:Broker
2024-01-01 open Assets:Cash

2024-01-02 * "Buy"
  Assets:Broker 5 AAPL {100 USD}
  Assets:Cash -500 USD

2024-02-01 * "Sell"
  Assets:Broker -10 AAPL {100 USD}
  Assets:Cash 1000 USD
```

交易仍会记账，批次变为负数。**修正方法**：检查记账行写出的数量和成本；漏记买入是常见原因。见[批次与成本](/zh-cn/guides/lots-and-cost-basis/)。

## CloseNonZeroAccount

*尝试关闭一个余额非零的账户*

`close` 指令关闭的账户，自身余额（不含子账户）在某种商品上不为零。

```zhang
2024-01-01 open Assets:Old-Card
2024-01-01 open Equity:Opening-Balances

2024-01-02 * "Opening balance"
  Assets:Old-Card 100 CNY
  Equity:Opening-Balances

2024-03-31 close Assets:Old-Card
```

账户仍会被关闭。**修正方法**：在 `close` 之前把剩余金额转到另一个账户。

## BudgetDoesNotExist

*预算不存在*

指令指定的[预算](/zh-cn/reference/directives/budget/)在其日期未定义：

- `budget-add`、`budget-transfer` 或 `budget-close`。这条指令会被忽略。
- 记到通过 `budget` 元数据关联到该预算的账户的记账行。每个账户和预算报告一次，带有 `account_name` 和 `budget_name` 元数据；该记账行不计入预算，交易仍会记账。

```zhang
2024-01-01 budget-add Travel 500 CNY
```

**修正方法**：添加 `budget` 指令，日期在首次使用之日或之前，或者改正预算的名称。

## DefineDuplicatedBudget

*尝试创建一个重复的预算*

`budget` 指令指定的预算已经存在。第二个定义会被忽略。

```zhang
2024-01-01 budget Food CNY
2024-02-01 budget Food CNY
```

**修正方法**：删除第二条 `budget` 指令，或者给它另起一个名字。

## MultipleOperatingCurrencyDetect

*账本中存在多项 operating currency 的配置，这是 zhang 中不推荐的用法*

`operating_currency` 选项被设置了不止一次。张记账只支持一种主货币；使用最后读到的值。Beancount 账本常常为 Fava 设置多个。

```zhang
option "operating_currency" "USD"
option "operating_currency" "EUR"
```

**修正方法**：只保留一个 `operating_currency` 选项。

## ParseInvalidMeta

*指令中存在无效的 meta 值*

张记账读取的某个元数据或选项，它的值无法理解：

- `open` 的 `booking_method` 元数据，或者 `default_booking_method` 选项，不是一种记账方法。账户按账本的默认记账方法记账，无效的选项使默认值保持为 `FIFO`；
- [`plugin`](/zh-cn/reference/directives/plugin/#能力) 指令的 `timeout` 或 `allowed_paths` 元数据无效。插件以该能力的默认值运行。

```zhang
2024-01-01 open Assets:Cash
  booking_method: "NON_EXIST"
```

账本仍会加载。**修正方法**：使用有效的值：记账方法为 `STRICT`、`FIFO` 或 `LIFO`；`timeout` 和 `allowed_paths` 见[插件的能力](/zh-cn/reference/directives/plugin/#能力)。

## UnsupportedBookingMethod

*暂不支持该预订方法，账户改用默认的预订方法*

账户的 `booking_method` 或 `default_booking_method` 选项，是张记账尚未实现的记账方法：`NONE`、`AVERAGE` 或 `AVERAGE_ONLY`。这个错误只在 `open` 或 `option` 指令上报告一次。账户按账本的默认记账方法记账，不受支持的选项使默认值保持为 `FIFO`。

```zhang
2024-01-01 open Assets:Broker
  booking_method: "AVERAGE"
```

**修正方法**：使用受支持的记账方法之一：`STRICT`、`FIFO` 或 `LIFO`。

## AmbiguousLotMatch

*STRICT 预订方法下，减仓匹配到多个批次，无法确定减哪一个*

在使用 `STRICT` 记账方法的账户上，一次减仓匹配到多个批次，又没有把它们全部减完，所以要减哪个批次是有歧义的。这遵循 Beancount 的 `STRICT` 方法。交易仍会记账，在匹配的批次中按 `FIFO` 处理，所以在歧义解决之前，账本的各项数字照常保留。错误的 `matched_lots` 元数据列出它匹配到的批次。

```zhang
2024-01-01 commodity USD
2024-01-01 commodity AAPL
2024-01-01 open Assets:Broker
  booking_method: "STRICT"
2024-01-01 open Assets:Cash

2024-01-02 * "Buy"
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -1000 USD
2024-02-01 * "Buy"
  Assets:Broker 10 AAPL {110 USD}
  Assets:Cash -1100 USD
2024-03-01 * "Sell"
  Assets:Broker -5 AAPL {}
  Assets:Cash 500 USD
```

**修正方法**：用成本和取得日期指明要减的批次，例如 `-5 AAPL {100 USD, 2024-01-02}`；一次减完所有匹配的批次；或者让账户使用 `FIFO` 或 `LIFO` 记账方法。

## PluginError

*插件 `<plugin>`：`<message>`*

用 [`plugin`](/zh-cn/reference/directives/plugin/) 指令声明的 WASM 插件报告了一个问题，报告者通常是只检查账本、不修改账本的校验插件。插件通过 `zhang_emit_error` 宿主函数报告问题，账本仍会加载。错误的 `message` 元数据描述问题，`plugin` 元数据指出插件；插件还可以加上自己的元数据。错误指向插件指定的指令；插件没有指定时，指向插件的 `plugin` 指令。[编写插件](/zh-cn/developers/writing-plugins/#报告错误)介绍了插件如何报告错误。

例如，一个要求每笔交易都有收款方的插件，会在下面这笔交易上报告 `message: "payee is missing"`：

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "lunch"
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
```

**修正方法**：按 `message` 元数据的要求处理，或者修改插件的设置。如果消息说插件用无效的数据调用了 `zhang_emit_error`，那是插件的缺陷：请报告给插件的作者。

## 账本无法加载的情况

以下问题会让张记账无法加载账本。`zhang serve` 以退出码 1 退出，并在日志中记录原因；设置环境变量 `RUST_LOG=info` 可以看到日志。服务器已在运行时，失败的重新加载会保持账本原样。

- 文件中的**语法错误**。消息会指出文件、行和列，例如 `failed to parse zhang file: unexpected input at line 4, column 3`。
- [`default_rounding`](/zh-cn/reference/directives/options/#default_rounding) 或 [`directive_output_path`](/zh-cn/reference/directives/options/#directive_output_path) 选项的**无效值**，或者[商品](/zh-cn/reference/directives/commodity/#舍入)的 `rounding` 元数据的无效值。消息为 `option value is invalid`。
- 启用插件时，**插件**的模块缺失或无法加载，或者插件调用失败或运行超过 `timeout`。见[插件](/zh-cn/reference/directives/plugin/#加载与顺序)。
- 超出[通配符的限制](/zh-cn/reference/directives/include/#通配符)的 **`include` 模式**。
