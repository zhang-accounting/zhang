---
title: 交易
description: 交易的参考，包括交易的记账行、金额、成本和价格，以及各自的元数据。
sidebar:
  order: 4
---

交易在账户之间转移金额。它由一个首行和若干缩进的记账行组成：首行写日期、可选的标记、收款方和摘要，之后每个记账行占一行。在每种商品上，记账行的金额之和必须为零。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] [<Flag>] ["<Payee>"] ["<Narration>"] [#tag …] [^link …]
  [<key>: <value>]
  <Account> [<Amount>] [<Cost>] [@ <Price> | @@ <TotalPrice>]
    [<key>: <value>]
  <Account> …
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 日期，后面可以跟一天中的时刻（`10:30` 或 `10:30:15`），按账本的[时区](/zh-cn/reference/directives/options/#timezone)理解。 |
| `<Flag>` | 否 | `*` 表示已完成的交易，`!` 表示需要核对的交易。见[标记](#标记)。 |
| `"<Payee>"`、`"<Narration>"` | 否 | 带引号的字符串。见[收款方与摘要](#收款方与摘要)。 |
| `#tag`、`^link` | 否 | 标签和链接，顺序任意，写在首行末尾。 |
| `<key>: <value>` | 否 | 交易的元数据，或者其上方记账行的元数据。见[元数据](#元数据)。 |
| `<Account>` | 是 | 记账行的账户。 |
| `<Amount>` | 否 | 一个数字加一种商品，例如 `-35.50 CNY`。可以有一个记账行省略它。 |
| `<Cost>` | 否 | 这些单位的成本：`{…}` 为单位成本，`{{…}}` 为总成本。见[成本与价格](#成本与价格)。 |
| `@ <Price>`、`@@ <TotalPrice>` | 否 | 这些单位换算时所用的价格，按单位或按总额。 |

记账行和元数据行紧跟在首行之后，中间不能有空行：空行会结束这笔交易。以 `;`、`#`、`*` 或 `//` 开头的缩进行是注释，记账行末尾也可以跟一个 `; comment` 注释。

## 示例

```zhang
2024-01-01 open Assets:Cash CNY
2024-01-01 open Expenses:Food CNY

2024-01-02 * "Cafe" "lunch" #work ^trip-2024
  Assets:Cash -35.50 CNY
  Expenses:Food 35.50 CNY

2024-01-03 12:30 * "Bakery" "bread"
  Assets:Cash -12 CNY
  Expenses:Food
```

## 首行

### 标记

| 标记 | 含义 |
|---|---|
| `*` | 已完成的交易。与 Beancount 一样，`txn` 是同一个标记。 |
| `!` | 需要核对的交易。 |
| `P` | 补齐交易，由 [`balance … with pad`](/zh-cn/reference/directives/balance/#用-with-pad-补齐) 添加。张记账把用 `P` 写的交易与余额断言一起排序：排在 `open` 和 `commodity` 之后、同一日期和时间的其他条目之前。 |
| 其他大写字母 | 你自己的标记，按原样保留。`C` 也是普通的标记。 |

没有标记的交易视为已完成（`*`）。

### 收款方与摘要

首行需要一个标记，或者至少一个带引号的字符串。

| 首行 | 收款方 | 摘要 |
|---|---|---|
| `* "Cafe" "lunch"` | `Cafe` | `lunch` |
| `* "lunch"` | 无 | `lunch` |
| `"Cafe" "lunch"` | `Cafe` | `lunch` |
| `"Cafe"` | `Cafe` | 无 |

有标记时，单个字符串是摘要。没有标记时，它是收款方。

## 记账行

### 金额

金额是一个数字后面跟一种商品，例如 `-1,234.50 CNY`。数字的各位之间可以有 `,` 或 `_`，数字也可以是带 `+`、`-`、`*`、`/` 和括号的表达式：`(120 + 35) / 2 CNY` 就是 `77.5 CNY`。商品必须在交易的日期已经[定义](/zh-cn/reference/directives/commodity/)。

### 省略金额

一笔交易中可以有一个记账行省略金额。张记账会给它一个使交易配平的金额：

- 其他记账行的权重（见[交易如何配平](#交易如何配平)）必须恰好在一种商品上不平衡。省略金额的记账行得到这个差额的相反数。
- 如果其他记账行已经配平，并且都使用同一种商品，它得到该商品的零。流水保留你写下的这个记账行。
- 推断出的金额是精确的。只有除法让金额超过 20 位小数时（例如把总成本分摊到 3 个单位上），才会按该商品的 `rounding` 舍入，舍入到该商品的精度与交易中该商品写出的最多小数位数两者中较大的那个。

没有可以推断的依据、有多个记账行省略金额、或者其他记账行在多种商品上不平衡时，交易会被报告并且**不记账**：在你修正它之前，它不改变任何余额。见[错误](#错误)。

### 成本与价格

| 记账行 | 权重 |
|---|---|
| `Assets:Wallet 100 USD` | `100 USD` |
| `Assets:Wallet 100 USD @ 7.10 CNY` | `710 CNY`：数量乘以单价 |
| `Assets:Wallet 100 USD @@ 710 CNY` | `710 CNY`：总价 |
| `Assets:Broker 10 AAPL {185 USD}` | `1850 USD`：数量乘以单位成本 |
| `Assets:Broker 3 AAPL {{1000 USD}}` | `1000 USD`：总成本 |
| `Assets:Broker -5 AAPL {}` | 减仓所扣除批次的成本 |

成本还可以给出批次的取得日期和标签，例如 `{185 USD, 2024-01-02, "lot-a"}`。同时带成本和价格的记账行，例如 `-5 AAPL {185 USD} @ 200 USD`，按成本计算权重；价格只记录它卖出的价格。批次如何建立、匹配和减少，见[批次与成本](/zh-cn/guides/lots-and-cost-basis/)和[记账方法](/zh-cn/reference/directives/account/#记账方法)。

### 交易如何配平

与 Beancount 一样，每种商品都必须各自配平。记账行贡献它的权重：它的数量、按价格换算的数量，或者按成本计算的数量（见上表）。张记账把每种商品的权重相加，按该商品的[舍入方式](/zh-cn/reference/directives/commodity/#舍入)舍入到它的[精度](/zh-cn/reference/directives/commodity/#精度)，结果必须为零。

使用默认精度 2 时，相差 `0.004 CNY` 的交易可以配平，相差 `0.006 CNY` 的不能。恰好相差 `0.005 CNY` 的交易在默认的 `RoundDown` 下可以配平，在 `RoundUp` 下不能。

不平衡的交易会报告为 [`UnbalancedTransaction`](/zh-cn/reference/error-codes/#unbalancedtransaction)，但仍会记账，所以账户余额与你写下的内容一致。

## 元数据

元数据行是 `key: value` 对。交易有自己的元数据，每个记账行也可以有自己的元数据：

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
    category: "meals"
```

这里 `invoice` 属于交易，`receipt` 属于 `Assets:Cash` 记账行，`category` 属于 `Expenses:Food` 记账行。

值是一个带引号的字符串，或者一个不含空格、引号、冒号、括号或逗号的单词，例如 `123`、`2024-01-01` 或 `TRUE`。账户名作为值时要加引号。所有值都保存为文本。

### 哪些行属于记账行

在 zhang 文件（`.zhang`）中，元数据行**只有缩进比上方记账行更深时**，才属于该记账行。其他所有元数据行都属于交易，不论它在哪里：在记账行之前、之间还是之后。

```zhang
2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 CNY
    receipt: "r-17"      ; 比记账行缩进更深：属于该记账行
  Expenses:Food 10 CNY
  invoice: "2024-001"    ; 与记账行缩进相同：属于交易
```

旧版本的张记账把交易元数据写在记账行之后，缩进与记账行相同，所以按照这条规则，现有 zhang 账本的含义保持不变。用制表符缩进时，一个制表符计到下一个四的倍数列。

在 Beancount 文件（`.bean`、`.bc` 或 `.beancount`）中，张记账改为遵循 Beancount 的规则：第一个记账行之前的元数据属于交易，而**记账行之后的每一行元数据都属于该记账行，不论缩进如何**。Beancount 和 Fava 就是这样读取文件的。

### 张记账如何写入元数据

张记账写入交易时（例如你在网页界面中创建或编辑交易时），先在首行之后写交易的元数据，然后依次写每个记账行，并在其后写该记账行自己的元数据，比记账行再缩进一级：

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
```

张记账在两种文件格式中以同样的方式读取这种布局，Beancount 和 Fava 也是如此。不是单个单词的键，例如 `"my key"`，会加上引号写出。Beancount 没有带引号的键，所以在 Beancount 账本中，网页界面只接受 Beancount 能读取的新键。

### 使用元数据

- 张记账的 API 把每个记账行的元数据随记账行一起返回，与交易自己的元数据并列；创建或编辑交易时也接受这两种元数据。
- 在[查询](/zh-cn/reference/query-language/#元数据函数)中，`meta('key')` 读取记账行的元数据，`entry_meta('key')` 读取交易的元数据，`any_meta('key')` 先读记账行的、再读交易的。postings 表的 `meta` 列以文本形式保存记账行的元数据。
- `document` 元数据把一个文件关联到交易，不论它写在交易上还是写在某个记账行上。见[文档](/zh-cn/reference/directives/document/)。

### 插件

WASM 插件收到和返回的交易中，每个记账行的元数据位于该记账行的 `meta` 字段。针对旧版本张记账（引入记账行元数据之前）构建的插件仍然可以工作，但它会读取并写回交给它的每一条指令，所以经过它的**每一笔**交易都会丢失记账行的元数据，而不只是它修改的那些。重新构建这样的插件，才能保留记账行的元数据。

## 错误

| 错误 | 触发条件 | 是否记账 |
|---|---|---|
| [`UnbalancedTransaction`](/zh-cn/reference/error-codes/#unbalancedtransaction) | 某种商品不平衡。 | 是 |
| [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine) | 交易配平所用的商品未定义。 | 是 |
| [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist)、[`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed) | 记账行的账户在交易日期不处于开立状态。 | 是 |
| [`NoEnoughCommodityLot`](/zh-cn/reference/error-codes/#noenoughcommoditylot)、[`AmbiguousLotMatch`](/zh-cn/reference/error-codes/#ambiguouslotmatch) | 按成本减仓时找不到对应的批次。 | 是 |
| [`BudgetDoesNotExist`](/zh-cn/reference/error-codes/#budgetdoesnotexist) | 记账行的账户关联到未定义的预算。 | 是 |
| [`TransactionCannotInferTradeAmount`](/zh-cn/reference/error-codes/#transactioncannotinfertradeamount) | 省略的金额没有可以推断的依据。 | 否 |
| [`TransactionHasMultipleImplicitPosting`](/zh-cn/reference/error-codes/#transactionhasmultipleimplicitposting) | 不止一个记账行省略了金额。 | 否 |
| [`TransactionExplicitPostingHaveMultipleCommodity`](/zh-cn/reference/error-codes/#transactionexplicitpostinghavemultiplecommodity) | 其他记账行在多种商品上不平衡。 | 否 |

## Beancount 兼容性

- 在 Beancount 文件中，一天中的时刻写成 `time: "HH:MM:SS"` 元数据。张记账也这样写入。
- Beancount 要求每笔交易都有标记或 `txn`。没有标记的首行，例如 `2024-01-02 "Cafe" "lunch"`，只在张记账中可用。
- 张记账不读取记账行前面的标记，例如 `! Assets:Cash -10 CNY`：含有这种写法的账本无法加载。
- Beancount 的 `pushtag` / `poptag` 和 `pushmeta` / `popmeta` 只能在 Beancount 文件中使用。张记账在读取文件时应用它们。
- 在 Beancount 文件中，记账行元数据遵循 Beancount 的规则，见[哪些行属于记账行](#哪些行属于记账行)。

## 相关页面

- [记录交易](/zh-cn/guides/recording-transactions/)：在文件和网页界面中编写交易。
- [批次与成本](/zh-cn/guides/lots-and-cost-basis/)：成本、批次和记账方法。
- [余额](/zh-cn/reference/directives/balance/)：用对账单核对结果。
