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
  [<Flag>] <Account> [<Amount>] [<Cost>] [@ <Price> | @@ <TotalPrice>] [; <comment>]
    [<key>: <value>]
  [<Flag>] <Account> …
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 日期，后面可以跟一天中的时刻（`10:30` 或 `10:30:15`），按账本的[时区](/zh-cn/reference/directives/options/#timezone)理解。 |
| `<Flag>` | 否 | 在首行上，`*` 表示已完成的交易，`!` 表示需要核对的交易。见[标记](#标记)。写在记账行的账户之前时，是记账行自己的标记。见[记账行标记](#记账行标记)。 |
| `"<Payee>"`、`"<Narration>"` | 否 | 带引号的字符串。见[收款方与摘要](#收款方与摘要)。 |
| `#tag`、`^link` | 否 | 标签和链接，顺序任意，写在首行末尾。 |
| `<key>: <value>` | 否 | 交易的元数据，或者其上方记账行的元数据。见[元数据](#元数据)。 |
| `<Account>` | 是 | 记账行的账户。 |
| `<Amount>` | 否 | 一个数字加一种商品，例如 `-35.50 CNY`。可以有一个记账行省略它。 |
| `<Cost>` | 否 | 这些单位的成本：`{…}` 为单位成本，`{{…}}` 为总成本。见[成本与价格](#成本与价格)。 |
| `@ <Price>`、`@@ <TotalPrice>` | 否 | 这些单位换算时所用的价格，按单位或按总额。 |

记账行和元数据行紧跟在首行之后，中间不能有空行：空行会结束这笔交易。以 `;`、`#` 或 `//` 开头的缩进行是注释，记账行末尾也可以跟一个 `; comment` 注释。`*` 不是注释符号：`* Assets:Cash -10 CNY` 这样的缩进行是[带标记的记账行](#记账行标记)，与 Beancount 相同。在 Beancount 文件中，`# Assets:Cash -10 CNY` 也是带标记的记账行，与 Beancount 的读法相同。

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
| `P` | 补齐交易，由 [`balance … with pad`](/zh-cn/reference/directives/balance/#用-with-pad-补齐) 或 [`pad`](/zh-cn/reference/directives/balance/#pad-指令) 添加，或者由网页界面在 Beancount 文件中为补齐写入。张记账把用 `P` 写的交易与余额断言一起排序：排在 `open` 和 `commodity` 之后、同一日期和时间的其他条目之前。`pad` 的补齐交易紧跟在它之后。 |
| 其他大写字母，或 `#`、`&`、`?`、`%` | 你自己的标记，按原样保留。`C` 也是普通的标记。这些是 Beancount 接受的其他标记。 |

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

### 记账行标记

与 Beancount 一样，记账行可以有自己的标记，写在账户之前，中间至少隔一个空格或制表符。导入工具和 Fava 用它标出需要核对的记账行：

```zhang
2024-01-10 * "Lunch"
  ! Assets:Cash -10 CNY
  Expenses:Food
```

记账行标记可以是除 `#` 以外的任何 Beancount 标记：`*`、`!`、`&`、`?`、`%` 或一个大写字母。在张记账文件中，它不能是 `#`：以 `#` 开头的缩进行在张记账文件中是注释，所以 `# Assets:Cash -10 CNY` 仍然是注释，不会入账。在 Beancount 文件中，记账行标记也可以是 `#`，与 Beancount 相同，这一行就是标记为 `#` 的记账行。`txn` 只能用作首行的标记。标记之后的空格是必需的。

:::caution[在张记账文件中 `*` 不是注释]
张记账文件的注释符号是 `;`、`#` 和 `//`。`*` 是标记，不是注释符号：以 `*` 开头、既不是记账行也不是指令的行是解析错误，而不是注释。旧版本的张记账会把这样的行当作注释。用 `*` 写注释的 `.zhang` 文件，例如 `* Banking` 这样的 org-mode 标题，必须改用 `;` 或 `#`，否则文件无法加载，并在该行报告解析错误。`.bean` 文件不受影响：在交易之外以 `*` 开头的行在 Beancount 中是 org-mode 标题，张记账与 Beancount 一样跳过它。
:::

记账行标记不影响记账行如何入账。张记账会保留它：

- 张记账写入这笔交易时，会把标记写回账户之前。在网页界面中编辑交易时，被编辑的每个记账行都保留自己的标记，虽然表单并不显示它。
- 在[查询](/zh-cn/reference/query-language/#列)中，`posting_flag` 列给出这个标记；记账行没有标记时为 `NULL`，与 beanquery 相同。`flag` 列是交易的标记。

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

值是一个带引号的字符串，或者一个不带引号的值：不含空格、引号、冒号、括号或逗号的单词，例如 `123`、`2024-01-01`、`USD` 或 `TRUE`；账户名，例如 `Assets:Bank`；金额，例如 `10.00 USD`；或者带分组分隔符或算式的数字，例如 `1,000` 或 `(1 + 2) * 3`。这些正是 Beancount 不加引号就能读取的值，所以使用它们的 Beancount 账本也能在张记账中加载。所有值都按书写原样保存为文本：`(1 + 2) * 3` 仍是 `(1 + 2) * 3`，写回时也保持不变。

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

插件也可以设置记账行的 `flag`。在张记账账本中，张记账文件无法保存的记账行标记 `#` 会在张记账写入这笔交易时被省略，例如在网页界面中编辑之后：这个记账行会不带该标记写入，从而仍然是记账行，而不会变成注释。`*` 标记会照常写入，与 Beancount 文件相同。

## 错误

| 错误 | 触发条件 | 是否记账 |
|---|---|---|
| [`UnbalancedTransaction`](/zh-cn/reference/error-codes/#unbalancedtransaction) | 某种商品不平衡。 | 是 |
| [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine) | 交易配平所用的商品未定义。 | 是 |
| [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist)、[`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed) | 记账行的账户在交易日期不处于开立状态。 | 是 |
| [`CommodityNotAllowed`](/zh-cn/reference/error-codes/#commoditynotallowed) | 记账行的数量属于其账户的 `open` 没有列出的商品。 | 是 |
| [`NoEnoughCommodityLot`](/zh-cn/reference/error-codes/#noenoughcommoditylot)、[`AmbiguousLotMatch`](/zh-cn/reference/error-codes/#ambiguouslotmatch) | 按成本减仓时找不到对应的批次。 | 是 |
| [`BudgetDoesNotExist`](/zh-cn/reference/error-codes/#budgetdoesnotexist) | 记账行的账户关联到未定义的预算。 | 是 |
| [`TransactionCannotInferTradeAmount`](/zh-cn/reference/error-codes/#transactioncannotinfertradeamount) | 省略的金额没有可以推断的依据。 | 否 |
| [`TransactionHasMultipleImplicitPosting`](/zh-cn/reference/error-codes/#transactionhasmultipleimplicitposting) | 不止一个记账行省略了金额。 | 否 |
| [`TransactionExplicitPostingHaveMultipleCommodity`](/zh-cn/reference/error-codes/#transactionexplicitpostinghavemultiplecommodity) | 其他记账行在多种商品上不平衡。 | 否 |

## Beancount 兼容性

- 在 Beancount 文件中，一天中的时刻写成 `time: "HH:MM:SS"` 元数据。张记账也这样写入。张记账与 Beancount 一样忽略 `balance` 和 `pad` 上的时刻。
- Beancount 要求每笔交易都有标记或 `txn`。没有标记的首行，例如 `2024-01-02 "Cafe" "lunch"`，只在张记账中可用。
- 记账行前面的标记，例如 `! Assets:Cash -10 CNY` 或 `* Assets:Cash -10 CNY`，按 Beancount 的方式读取。见[记账行标记](#记账行标记)。`# Assets:Cash -10 CNY` 在 Beancount 文件中是带标记的记账行，在张记账文件中是注释。Beancount 也接受不带空格的标记，例如 `!Assets:Cash -10 CNY`；张记账会把这一行报告为错误。
- 在交易之外以 `*` 开头的行（org-mode 标题）在 Beancount 文件中会被跳过，与 Beancount 相同。在张记账文件中它是解析错误：请用 `;` 或 `#` 写注释。
- Beancount 的 `pushtag` / `poptag` 和 `pushmeta` / `popmeta` 只能在 Beancount 文件中使用。张记账在读取文件时应用它们。
- 在 Beancount 文件中，记账行元数据遵循 Beancount 的规则，见[哪些行属于记账行](#哪些行属于记账行)。

## 相关页面

- [记录交易](/zh-cn/guides/recording-transactions/)：在文件和网页界面中编写交易。
- [批次与成本](/zh-cn/guides/lots-and-cost-basis/)：成本、批次和记账方法。
- [余额](/zh-cn/reference/directives/balance/)：用对账单核对结果。
