---
title: 从 Beancount 迁移
description: 用张记账运行已有的 Beancount 账本，并了解它对 Beancount 的支持程度、两者的行为差异以及各自独有的功能。
sidebar:
  order: 4
---

张记账可以直接读取 Beancount 账本。你可以把它指向你的 `main.bean`，照旧编辑你的文件：张记账在网页界面中显示它们，你在网页界面中记录的内容会以 Beancount 语法写回。

本页介绍如何运行 Beancount 账本，并在[兼容性](#兼容性)一节中列出张记账与 Beancount 的不同之处。

## 运行 Beancount 账本

把账本目录和主文件名交给 `zhang serve`：

```shell
zhang serve /path/to/ledger --endpoint main.bean
```

使用 Docker 时，把目录挂载到 `/data`，并在镜像名之后加上 `--endpoint`：

```shell
docker run --name zhang -v "/path/to/ledger:/data" -p "8000:8000" kilerd/zhang:latest --endpoint main.bean
```

- 主文件以 `.bean`、`.bc` 或 `.beancount` 结尾时，张记账会把整个账本都按 Beancount 读取：主文件以及它引入的每个文件，无论扩展名是什么。一个账本要么是 Beancount 语法，要么是张记账语法，不能混用。
- 打开 `http://localhost:8000`。侧边栏中的**总览**项显示张记账发现了多少问题，总览页面列出这些问题。请先逐一处理它们：大部分来自下文所述的差异。
- 你在网页界面中记录的内容会写入一个与主文件扩展名相同的文件，默认是 `data/{{year}}/{{month_str}}.bean`，使用 Beancount 语法。见[记录交易](/zh-cn/guides/recording-transactions/#新条目写入的位置)。

[安装](/zh-cn/getting-started/installation/)列出了 `zhang serve` 的所有选项。

## 记账行元数据

张记账按 Beancount 和 Fava 的方式读取 Beancount 文件中交易的元数据：第一条记账行之前的元数据属于交易，记账行之后的每一行元数据都属于该记账行，无论缩进多少。

```beancount
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"     ; 属于交易
  Assets:Cash -10 CNY
    receipt: "r-17"       ; 属于 Assets:Cash 记账行
  Expenses:Food 10 CNY
  category: "meals"       ; 属于 Expenses:Food 记账行
```

旧版本的张记账把交易的元数据写在记账行之后。这样的文件现在在张记账中的读法不同了：这些元数据属于最后一条记账行，Fava 一直是这样读取的。有两个键仍照旧工作：

- `time`：写在最后一条记账行之后、与记账行缩进相同的 `time`（旧版本张记账就是这样写的），仍然是交易的时间，除非交易自己有 `time`，或者其他记账行有 `time`。缩进比其记账行更深的 `time` 仍属于该记账行。
- `document`：记账行上的文档是其所属交易的文档，因此附加到交易上的文档仍然附加在交易上。

张记账写入交易时，例如你在网页界面中编辑交易之后，会把交易的元数据写在记账行之前，并把每条记账行的元数据紧跟在该记账行之下，因此文件在张记账、Beancount 和 Fava 中的读法都一样。见[交易](/zh-cn/reference/directives/transaction/)。

## 兼容性

### 指令

| Beancount | 在张记账中 |
|---|---|
| `open` | 读取，包括其商品和记账方法（`open Assets:Broker HOOL "FIFO"`）。这些商品必须已声明，但使用其他商品的记账行不会被报告。 |
| `close` | 读取。关闭仍有余额的账户会被报告为 [`CloseNonZeroAccount`](/zh-cn/reference/error-codes/#closenonzeroaccount)。 |
| `commodity` | 读取，并且是必需的：见[商品必须声明](#商品必须声明)。 |
| 交易 | 读取，支持标记 `*`、`!` 和其他 Beancount 标记（在交易上和[记账行上](/zh-cn/reference/directives/transaction/#记账行标记)）、`txn` 关键字、标签、链接、成本（`{}`、`{{}}`，可带日期和批次标签）以及价格（`@`、`@@`）。 |
| `balance` | 读取，可以带 `~` 容差。没有容差时精确匹配：见[余额断言是精确的](#余额断言是精确的)。 |
| `pad` | 读取，并与它所服务的 `balance` 配对：见[补齐](#补齐)。 |
| `note`、`event` | 读取。 |
| `document` | 读取。路径与 Beancount 一样相对于该指令所在的文件：见[文档路径](#文档路径)。 |
| `price` | 读取，用于查询和货币页面中的估值。 |
| `query` | 读取：这些查询出现在[查询](/zh-cn/guides/querying/)页面的**已保存**菜单中。 |
| `custom` | 读取。`custom budget …` 定义[预算](/zh-cn/guides/budgets/)：见[预算](#预算)。 |
| `option` | 读取。只有部分选项起作用：见[选项](#选项)。 |
| `plugin` | Python 插件不会运行：见[插件](#插件)。 |
| `include` | 读取，包括 `*` 模式，例如 `include "2024/*.bean"`。 |
| `pushtag`、`poptag`、`pushmeta`、`popmeta` | 读取。 |

`time: "HH:MM:SS"` 元数据可以为指令加上一天中的时间，但 `balance` 和 `pad` 除外：张记账与 Beancount 一样忽略它们的时间，见[余额断言的时间](#余额断言的时间)。

Beancount 不加引号就能读取的元数据值——账户、货币、数字或算式、金额、日期、标签、`TRUE`、`FALSE` 和 `NULL`——在交易、记账行和其他所有指令上都按书写原样读取并保存为文本。`counterpart: Assets:Bank` 是文本 `Assets:Bank`，`limit: 10.00 USD` 是文本 `10.00 USD`，`1 + 2` 仍是 `1 + 2`，而 Beancount 会算出 `3`。标签值保留它的 `#`，`TRUE`、`FALSE` 和 `NULL` 保持原词，而 beanquery 显示的是 `True`、`False` 和空值。张记账写回这些值时保持原样。

张记账无法读取以下内容。使用了它们的文件完全无法加载：

- 只有日期或只有批次标签的成本，例如 `{2024-01-01}` 或 `{"lot-1"}`。请先写成本：`{100.00 USD, 2024-01-01}`；
- 复合成本 `{100 # 9.95 USD}`，以及 `{*}`。

### 选项

张记账使用 Beancount 账本的以下选项：

- `title`，显示在网页界面中；
- `operating_currency`，但只能有一个：张记账保留最后一个，并把其余的都报告为 [`MultipleOperatingCurrencyDetect`](/zh-cn/reference/error-codes/#multipleoperatingcurrencydetect)；
- `account_previous_balances`、`account_previous_earnings`、`account_previous_conversions`、`account_current_earnings`、`account_current_conversions` 和 `conversion_currency`，用于查询的[会计期间](/zh-cn/reference/query-language/#会计期间)。

其他所有 Beancount 选项，例如 `booking_method`、`inferred_tolerance_default` 或 `documents`，都会列在设置页面上，除此之外被忽略。张记账也有自己的选项，例如 `default_booking_method` 和 `timezone`，Beancount 同样会忽略它们。见[选项](/zh-cn/reference/directives/options/)。

### 行为差异

#### 商品必须声明

记账行计算权重所用的每种商品、`price` 提到的每种商品以及 `open` 列出的每种商品，都必须有一条 `commodity` 指令。主货币由它的选项声明。Beancount 不要求 `commodity` 指令；张记账会把缺少的声明报告为 [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine)。请为每种商品添加一条，日期早于它的首次使用：

```beancount
1970-01-01 commodity HOOL
```

#### 余额断言是精确的

Beancount 允许 `balance` 在差额不超过某个容差时通过，这个容差由小数位数推导而来：`balance Assets:A 10.00 USD` 在余额为 `10.004 USD` 时通过。张记账不这样做：只有金额完全相等，或在你写在 `~` 之后的容差之内（例如 `10.00 ~ 0.005 USD`），断言才成立。见[余额](/zh-cn/guides/balances/#精确匹配与容差)。

#### 交易按商品的精度平衡

当一笔交易在每种商品上的权重之和按该商品的精度舍入后为零时，交易就是平衡的。精度是其 `commodity` 指令的 `precision` 元数据，没有时为 2 位小数。Beancount 则根据交易中写出的数字推导这个容差。请为金额更精细的商品（例如 `BTC`）设置自己的 `precision`。见[批次与成本](/zh-cn/guides/lots-and-cost-basis/#舍入)。

#### 记账方法

除非另行指定，张记账按 `FIFO` 记账，而 Beancount 的默认方法是 `STRICT`。要得到 Beancount 的行为，请添加 `option "default_booking_method" "STRICT"`；Beancount 自己的 `booking_method` 选项不会被读取。`NONE`、`AVERAGE` 和 `AVERAGE_ONLY` 尚未实现：使用其中之一的账户会得到一个错误，并按默认方法记账。批次标签与 Beancount 一样用于选择批次：写成 `{, "first"}` 的卖出会扣减标签为 `first` 的批次。见[批次与成本](/zh-cn/guides/lots-and-cost-basis/#选择记账方法)。

#### 补齐

张记账像 Beancount 一样，把每条 `pad` 与它所服务的 `balance` 条目配对：该账户在每种商品上、日期更晚的下一条 `balance`，直到该账户的下一条 `pad` 为止，但绝不包括与 `pad` 同一天的 `balance`。补齐交易的日期是 `pad` 的日期，`pad` 和它的 `balance` 可以在不同的文件中，不服务任何 `balance` 的 `pad` 会报告为 [`UnusedPad`](/zh-cn/reference/error-codes/#unusedpad)。不同之处在于：

- 补齐总是使账户精确等于断言金额，即使差额在明确写出的 `~` 容差之内，而 Beancount 此时不补齐。
- 只有对被补齐账户本身的断言会使用这条 `pad`：Beancount 还允许子账户上的断言用掉其父账户的 `pad`。
- 同一账户在不同文件中同一天的两条 `pad`，由张记账排在最后的那条补齐，它可能不是 Beancount 使用的那条。

见 [`pad` 指令](/zh-cn/guides/balances/#pad-指令)；父账户与子账户的情况见 [`balance`](/zh-cn/reference/directives/balance/#beancount-兼容性)。

#### 余额断言的时间

张记账与 Beancount 一样，在日期开始时、当天的交易之前检查 `balance`，并忽略它的 `time` 元数据。早期版本的张记账会在那个时刻检查它，即在当天该时刻之前的交易之后：当这改变了一条余额断言所检查的金额时，它会附带一条 [`BalanceTimeIgnored`](/zh-cn/reference/error-codes/#balancetimeignored) 提示。要在当天的交易之后检查，请把这条余额断言的日期写成下一天。

#### 文档路径

张记账与 Beancount 一样，相对于 `document` 所在的文件读取它的路径，你上传的文档也是这样写入的。早期版本的张记账把上传文档的路径写成相对于账本根目录，写入 `data/2026/10.bean` 这类文件中，Beancount 会报告这些文件不存在。张记账仍然能打开这些文档。在本地磁盘上，它会在每一条上附带一条 [`DocumentPathRelativeToRoot`](/zh-cn/reference/error-codes/#documentpathrelativetoroot) 提示，给出应改写成的路径，并把在任何位置都找不到的文档报告为 [`DocumentNotFound`](/zh-cn/reference/error-codes/#documentnotfound)。见[文档](/zh-cn/reference/directives/document/#路径)。

#### 价格

只有 `price` 指令会给出价格。用 `@` 和 `@@` 写在记账行上的价格不会，而在 Beancount 中使用 `implicit_prices` 插件时它们会。

#### 其他检查

- 关闭仍有余额的账户会被报告为错误。Beancount 允许这样做。
- 使用账户 `open` 中未列出的商品的记账行不会被报告。Beancount 会报告它们。
- 账户名必须以 `Assets`、`Liabilities`、`Equity`、`Income` 或 `Expenses` 开头。Beancount 中用来重命名它们的 `name_assets`… 等选项不会被读取。
- 在带引号的字符串中，不构成转义的反斜杠会被保留：`"\d"` 仍是 `\d`，而 Beancount 会丢掉这个反斜杠。

### 不支持的功能

#### 插件

Beancount 的插件是 Python 代码，张记账不运行它们，包括 Beancount 自带的插件，例如 `auto_accounts` 或 `implicit_prices`。没有它们时：

- 显式开设每个账户，而不是依赖 `auto_accounts`；
- 检查其他插件替你做了什么，然后在账本中或用[查询](/zh-cn/guides/querying/)完成这些工作。

只要账本没有启用张记账自己的插件，`plugin` 行就不会造成影响：使用 `option "features.plugin" "true"` 时，张记账会尝试把每个 `plugin` 都作为 WASM 模块加载，Python 模块名会让整个账本无法加载。张记账的插件是 WASM 模块：见[插件](/zh-cn/guides/plugins/)。

#### 预算

张记账读取 `custom budget Food CNY`、`custom budget-add Food 2000 CNY`、`custom budget-transfer Fun Food 300 CNY` 和 `custom budget-close Food`，其中 `budget` 一词和预算名都不加引号，张记账也以这种方式写入预算。Beancount 本身会拒绝这些行，因为它要求类型和名称都加引号；而加了引号的 `custom "budget" "Food" CNY` 对张记账来说是普通的 `custom` 指令。因此，如果你还用 `bean-check` 检查账本，它会把这些预算行报告为错误。

### 张记账独有的功能

以下功能在张记账中可用，在 Beancount 中没有对应：

- [预算](/zh-cn/guides/budgets/)，如上所述；
- 条目上的时间，Beancount 账本把它记在 `time` 元数据中；
- 张记账的 [WASM 插件](/zh-cn/guides/plugins/)及其[查询语言](/zh-cn/reference/query-language/)扩展，例如 `#budgets` 和 `#errors` 表。

张记账账本（`main.zhang`）还有更多自己的语法，例如 `balance … with pad` 和带时间的日期；Beancount 无法读取它。张记账没有在两种语法之间转换的命令，所以如果你想继续使用 Beancount 的工具，请让账本保持 Beancount 语法。
