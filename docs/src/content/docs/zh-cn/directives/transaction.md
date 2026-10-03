---
title: 交易
description: 编写交易、交易的分录以及各自的元数据。
---

# 交易

交易在账户之间转移金额。它由一行交易头（日期、可选的标记、收款人和描述）以及其下每条分录各占一行的缩进行组成。

## 基本语法

```zhang
{DATE} {FLAG} "{PAYEE}" "{NARRATION}" #{TAG} ^{LINK}
  {ACCOUNT} {AMOUNT} {COMMODITY}
  {ACCOUNT} {AMOUNT} {COMMODITY}
```

```zhang
2024-01-02 * "Cafe" "lunch" #trip
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
```

- 标记 `*` 表示已完成的交易，`!` 表示需要核对的交易。标记后只有一个字符串时，它是描述（narration）。
- 可以有一条分录省略金额：它取使交易平衡的金额。
- 交易内以 `;`、`#`、`*` 或 `//` 开头的行是注释。

## 元数据

元数据行是 `key: value` 形式的键值对。交易有自己的元数据，每条分录也可以有自己的元数据：

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
    category: "meals"
```

这里 `invoice` 属于交易，`receipt` 属于 `Assets:Cash` 分录，`category` 属于 `Expenses:Food` 分录。

### 哪些行属于分录

在 zhang 文件（`.zhang`）中，元数据行**只有在缩进比它上方的分录行更深时**才属于该分录。其他元数据行都属于交易，无论它在分录之前、之间还是之后。

```zhang
2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 CNY
    receipt: "r-17"      ; 比分录缩进更深：属于分录
  Expenses:Food 10 CNY
  invoice: "2024-001"    ; 与分录缩进相同：属于交易
```

旧版本的张记账会把交易元数据写在分录之后，缩进与分录相同，因此已有账本的含义保持不变。使用 Tab 缩进时，一个 Tab 计到下一个四列的倍数。

在 Beancount 文件（`.bean`、`.bc` 或 `.beancount`）中，张记账遵循 Beancount 的规则：第一条分录之前的元数据属于交易，**分录之后的每一行元数据都属于该分录，无论缩进多少**。Beancount 和 Fava 也是这样读取文件的。旧版本张记账写在分录之后的交易元数据，因此会成为最后一条分录的元数据，与 Fava 的读法一致；例外是 `time`：像旧版本张记账那样写在最后一条分录之后、与分录缩进相同的 `time`，仍作为交易的时间，除非交易自己有 `time` 或其他分录也有 `time`；缩进比分录更深的 `time` 仍属于分录。

### 张记账如何写入元数据

张记账写入交易时（例如在网页中新建或编辑交易），先在交易头之后写交易的元数据，然后依次写每条分录，分录自己的元数据紧跟其后，缩进再深两级：

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
```

张记账在两种文件格式中都以同样的方式读取这种布局，Beancount 和 Fava 也是如此。不是单个词的键（例如 `"my key"`）会加引号写入。Beancount 不支持带引号的键，因此在 Beancount 账本中，网页只接受 Beancount 能读取的新键。

### 使用元数据

- 张记账的 API 在每条分录中返回该分录的元数据，与交易自己的元数据并列；新建或编辑交易时也接受这两种元数据。
- 在[查询](/zh-cn/user-guide/query-language/#元数据函数)中，`meta('key')` 读取分录的元数据，`entry_meta('key')` 读取交易的元数据，`any_meta('key')` 先读分录、再读交易。postings 表的 `meta` 列以文本形式给出分录的元数据。
- `document` 元数据会把文件关联到交易，无论它写在交易上还是某条分录上。

## 插件

WASM 插件收到和返回的交易中，每条分录的元数据位于该分录的 `meta` 字段。基于旧版本张记账（尚无分录元数据时）构建的插件仍然可以使用，但它会读取并重新写出交给它的每一条指令，因此**每一笔**经过它的交易都会丢失分录的元数据，而不只是它修改过的交易。重新构建插件即可保留。
