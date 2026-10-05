---
title: 错误码
description: 张记账为账本报告的每个错误码，触发它的原因以及修正方法。
sidebar:
  order: 15
---

张记账加载账本时，会把发现的问题报告为错误。账本仍会加载：网页界面的错误页面列出这些错误，以及引发每个错误的指令；`GET /api/errors` 和 [`#errors`](/zh-cn/reference/query-language/#错误表) 查询表也是如此。错误先按文件、再按在文件中的位置排列，错误页面以文件和指令的字节偏移表示每个错误所在的位置。每个错误都有一个错误码，下面列出每个错误码，以及错误页面在中文界面中为它显示的提示。

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

成本无法确定时，也会报告这个错误：买入同时省略成本和现金金额、缺少多个成本、推导需要除以零或得到负成本，或者用 `{}` 减持却没有匹配的成本批次。无法匹配的减持还会报告 [`NoEnoughCommodityLot`](#noenoughcommoditylot)。整笔交易会被拒绝，之前的持仓保持不变。

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

省略金额的记账行，是在其他记账行匹配批次之后，根据它们的权重推断的。无法匹配的 `{}` 减持会在 [`NoEnoughCommodityLot`](#noenoughcommoditylot) 之后报告 [`TransactionCannotInferTradeAmount`](#transactioncannotinfertradeamount)，因为无法确定其成本。

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

## UnusedPad

*填充没有被该账户之后的任何余额断言使用*

一条 [`pad`](/zh-cn/reference/directives/balance/#pad-指令) 没有补齐任何金额：之后没有该账户的余额断言需要它。Beancount 报告同样的错误（“Unused Pad entry”）。一条 `pad` 为该账户在每种商品上、日期晚于这条 `pad` 的第一条 `balance` 服务，直到该账户的下一条 `pad` 为止；当每条这样的断言都已经成立、之后的日子里没有该账户的断言，或者该账户的另一条 `pad` 先取代了它时，它就是未使用的。

```zhang
; Assets:Checking 持有 100 USD
2024-01-01 pad Assets:Checking Equity:Opening-Balances
2024-01-02 balance Assets:Checking 100 USD
```

**修正方法**：删除这条 `pad`，或者把它移到它要服务的断言之前。与 `pad` 同一天的 `balance` 排在它之前，不论它们的时刻，都不会被补齐。

## PadWithCost

*填充的商品在该账户中按成本持有：补齐交易不带成本记账*

一次补齐要补齐的商品，由它的账户或其某个子账户按成本持有，即在带成本的批次中，例如以 `{100 USD}` 买入的股票。补齐交易仍会记账，但不带成本，错误报告在它所服务的余额断言上，与 Beancount 报告的 “Attempt to pad an entry with cost” 相同。张记账为这条断言报告一次；Beancount 为每个按成本持有的批次各报告一次。

```zhang
2024-01-02 * "Buy"
  Assets:Broker:Stock 10 AAPL {100 USD}
  Assets:Broker:Cash -1000 USD
2024-01-03 pad Assets:Broker:Stock Equity:Opening-Balances
2024-01-04 balance Assets:Broker:Stock 15 AAPL
```

**修正方法**：用一笔写明成本的交易记入缺少的数量，而不是补齐它们。

## BalanceTimeIgnored

*余额断言在其日期开始时检查：与 beancount 一样忽略它的时间*

这是一条提示，而不是账本的错误：它与错误列在一起，但余额断言是否成立与它无关。它报告在 Beancount 账本中这样一条 `balance` 上（只报告一次）：它的检查含义与早期版本的张记账不同。张记账与 Beancount 一样，在日期开始时、当天所有交易之前检查 Beancount 账本中的 `balance`，并忽略它的 `time` 元数据。早期版本会读取写成 `H:M:S` 的 `time`，在那个时刻检查余额，即在当天该时刻之前的交易之后。当这些交易改变了该账户及其子账户在该商品上的持有量时，就会给出这条提示：这条断言现在检查的是另一个金额，为它服务的 `pad` 补齐的也是另一个金额。早期版本不读取的 `09:30` 这样的时刻不改变任何东西，其他商品的交易或合计为零的交易也不会。

```beancount
2024-03-05 * "breakfast"
  Assets:Cash -10 CNY
  Expenses:Food
  time: "08:00:00"
2024-03-05 balance Assets:Cash 100 CNY
  time: "09:30:00"
```

**修正方法**：去掉 `time` 后，这条提示就会消失。要在当天的交易之后断言余额，把 `balance` 的日期写成下一天并去掉它的 `time`，网页界面就是这样写的。要在这些交易之前断言余额，去掉 `time` 即可。

## DocumentPathRelativeToRoot

*beancount 相对于 `<file>` 解析这个路径；请写成 `<path>`*

这是一条提示，而不是账本的错误，与 [`BalanceTimeIgnored`](#balancetimeignored) 一样：文档照常列出、照常打开。它报告在 Beancount 账本中这样一条 [`document`](/zh-cn/reference/directives/document/) 上：它的路径相对于 `document` 所在的文件（Beancount 查找的位置）找不到文件，但相对于账本根目录能找到。早期版本的张记账就是这样把你上传的文档写入 `data/2026/10.bean` 这类文件的，Beancount 会报告 “File does not exist”。张记账继续使用相对于账本根目录找到的文件，提示在它的 `file` 和 `written_as` 元数据中给出应改写成的路径。只有本地磁盘上的账本会给出这条提示；远程数据源见 [`DocumentNotFound`](#documentnotfound)。

```beancount title="data/2026/10.bean"
2026-10-04 document Assets:Bank "attachments/3f2a/statement.pdf"
```

**修正方法**：写成提示给出的、相对于文件的路径：这里是 `"../../attachments/3f2a/statement.pdf"`。提示随之消失，Beancount 也能找到文件。现在上传的文档就是这样写的。

## DocumentNotFound

*文档文件 `<path>` 不存在*

Beancount 账本中的一条 `document` 指向的文件不存在：相对于 `document` 所在的文件（Beancount 查找的位置）不存在，相对于账本根目录也不存在。Beancount 报告为 “File does not exist”。只有加载本地磁盘上的账本时，张记账才会查找这些文件，因为这样做代价很小：在 S3、WebDAV 或 GitHub 等远程数据源上，既不报告这个错误，也不报告 [`DocumentPathRelativeToRoot`](#documentpathrelativetoroot)。在那里，文档在你打开它时才查找：先相对于它所在的文件，再相对于账本根目录，两处都找不到时，打开它会得到文件不存在的回答。

```beancount title="data/2026/10.bean"
; 没有 data/2026/statement.pdf
2026-10-04 document Assets:Bank "statement.pdf"
```

**修正方法**：把文件放到路径所指的位置，或者改正相对于 `document` 所在文件的路径。

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

## CommodityNotAllowed

*货币 `<commodity>` 不允许用于账户 `<account>`：该账户的 open 指令限定了其它货币*

账户开立时列出了商品，而有东西把另一种商品放了进去。与 Beancount 一样，这样的 [`open`](/zh-cn/reference/directives/account/#商品) 把账户限定在它列出的商品之内。错误会报告在：

- 数量属于另一种商品的记账行上。只看数量：记账行的成本和价格不检查，列出 `AAPL` 的账户可以买入 `AAPL {90 EUR}`。省略金额的记账行，按张记账为它推断出的商品检查；
- 断言另一种商品的 [`balance`](/zh-cn/reference/directives/balance/) 上，断言零也一样；
- [`pad`](/zh-cn/reference/directives/balance/) 的补齐交易记入另一种商品时，报告在这条 `pad` 上，其两个账户中每个没有列出该商品的账户各报告一次。`balance … with pad` 是一条指令：它的账户只报告一次，断言和补齐都算在内；补齐来源账户没有列出该商品时，再为它报告一次。

错误带有 `account_name` 和 `commodity` 元数据，每个写下的记账行各报告一次。没有列出商品的 `open` 允许任何商品，限制只针对账户本身，子账户不受限制。账户被重新开立时，以该指令之前最近一次 `open` 的商品为准。

```zhang
2024-01-01 commodity USD
2024-01-01 commodity EUR
2024-01-01 open Assets:Bank USD
2024-01-01 open Equity:Opening

2024-01-10 * "Deposit in the wrong currency"
  Assets:Bank 100 EUR
  Equity:Opening -100 EUR
```

账本仍会加载，交易仍按写下的内容记账。**修正方法**：用 `open` 列出的商品记账，或者把这种商品加进 `open`：`2024-01-01 open Assets:Bank USD, EUR`。不写列表则允许任何商品。

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

像上例那样写了明确成本时，交易仍会记账，批次变为负数。使用 `{}` 时，整笔交易会被拒绝，还会报告 [`TransactionCannotInferTradeAmount`](#transactioncannotinfertradeamount)，之前的持仓保持不变。**修正方法**：检查记账行写出的数量和成本；漏记买入是常见原因。见[批次与成本](/zh-cn/guides/lots-and-cost-basis/)。

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
- [`plugin`](/zh-cn/reference/directives/plugin/#能力) 指令的 `timeout`、`allowed_paths` 或 `stage` 元数据无效。插件以该能力的默认值运行。

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

## CostMergingNotSupported

*暂不支持成本合并，该记账行按不带 `*` 的成本记账*

记账行的成本带有 `*`，即 Beancount 的成本合并标记，例如 `-5 AAPL {*}`。在 Beancount 中它表示在减仓之前先把账户的批次按平均成本合并，Beancount 自己也报告为尚未支持。张记账对该记账行报告一次，并按没有 `*` 的成本记账：`{*}` 像 `{}` 一样按账户的[记账方法](/zh-cn/reference/directives/account/#记账方法)选择批次，`{*, "lot-a"}` 像 `{"lot-a"}`。写回文件时保留 `*`。

```zhang
2024-01-01 commodity USD
2024-01-01 commodity AAPL
2024-01-01 open Assets:Broker
2024-01-01 open Assets:Cash
2024-01-01 open Income:Gains

2024-01-02 * "buy"
  Assets:Broker 10 AAPL {185 USD}
  Assets:Cash -1850 USD

2024-02-01 * "sell"
  Assets:Broker -5 AAPL {*}
  Assets:Cash 1000 USD
  Income:Gains
```

账本仍会加载，交易也会记账。**修正方法**：写出要扣减批次的成本，或者写 `{}` 交给记账方法选择。平均成本需要 `AVERAGE` 记账方法，它同样尚未实现，见 [`UnsupportedBookingMethod`](#unsupportedbookingmethod)。

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

以下问题会让张记账无法加载账本。`zhang serve` 会在标准错误输出中打印原因并以退出码 1 退出。服务器已在运行时，失败的重新加载会保持账本原样，在日志中记录原因，并在网页界面中显示原因，直到某次重新加载成功为止。

- 文件中的**语法错误**。消息会指出文件、行和列，例如 `failed to parse zhang file: unexpected input at line 4, column 3`。
- [`default_rounding`](/zh-cn/reference/directives/options/#default_rounding) 或 [`directive_output_path`](/zh-cn/reference/directives/options/#directive_output_path) 选项的**无效值**，或者[商品](/zh-cn/reference/directives/commodity/#舍入)的 `rounding` 元数据的无效值。消息为 `option value is invalid`。
- 启用插件时，**插件**的模块缺失或无法加载，或者插件调用失败或运行超过 `timeout`。见[插件](/zh-cn/reference/directives/plugin/#加载与顺序)。
