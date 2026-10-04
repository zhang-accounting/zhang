---
title: 文档
description: document 指令的参考，它把收据、对账单等文件关联到一个账户。
sidebar:
  order: 7
---

`document` 指令在某一天把一个文件（例如银行对账单、收据或合同）关联到一个账户。`document` 元数据则把文件关联到一笔交易。张记账在网页界面的文档页面列出这两种文档，并可以从那里打开文件。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] document <Account> "<Path>" [#tag …] [^link …]
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 文档的日期，可以附带一天中的时刻。 |
| `<Account>` | 是 | 文档所属的账户。 |
| `"<Path>"` | 是 | 文件的路径：在张记账文件中相对于账本根目录，在 Beancount 文件中相对于该指令所在的文件。见[路径](#路径)。 |
| `#tag`、`^link` | 否 | 标签和链接，与交易上的写法相同。 |

指令下方可以写元数据行。

要把文件关联到交易，给交易加上 `document` 元数据。多个文件就重复这个键。这项元数据也可以写在某个记账行上：它仍然把文件关联到这笔交易。

```text
YYYY-MM-DD * "<Payee>" "<Narration>"
  document: "<Path>"
  <postings>
```

## 示例

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Expenses:Food CNY

2024-01-31 document Assets:Bank:Checking "statements/2024-01.pdf" #statement
  bank: "ACME Bank"

2024-02-03 * "Cafe" "team lunch"
  document: "receipts/2024-02-03-cafe.jpg"
  Assets:Bank:Checking -120.00 CNY
  Expenses:Food
```

## 行为

- 文档不改变任何余额。
- 文件通过账本的[数据源](/zh-cn/deployment/data-sources/local/)读取，所以存放在 S3、WebDAV 或 GitHub 上的账本，文档的用法完全相同。

### 路径

- **在张记账文件中**，相对路径相对于账本根目录，即启动 `zhang serve` 时指定的目录，不论指令写在哪个文件中。在上面的示例中，文件是账本根目录下的 `statements/2024-01.pdf` 和 `receipts/2024-02-03-cafe.jpg`。
- **在 Beancount 文件中**，相对路径与 Beancount 的读法一样，相对于该指令所在文件的目录：`data/2026/10.bean` 中的 `"../../attachments/a.pdf"` 就是账本根目录下的 `attachments/a.pdf`。如果按两种读法都能找到文件，使用相对于文件的那个。
- 早期版本的张记账把你上传的文档写入 Beancount 文件时，使用的是相对于账本根目录的路径。张记账仍然能在那里找到它们。在本地磁盘上，它会在每一条上附带一条 [`DocumentPathRelativeToRoot`](/zh-cn/reference/error-codes/#documentpathrelativetoroot) 提示，给出应改写成的路径，并把 Beancount 文件中在任何位置都找不到的文档报告为 [`DocumentNotFound`](/zh-cn/reference/error-codes/#documentnotfound)。在远程数据源上，张记账加载账本时不做任何检查：打开文档时先相对于它所在的文件查找，再相对于账本根目录查找。
- 交易的 `document` 元数据在两种文件中都相对于账本根目录。
- 在张记账文件中，张记账加载账本时不检查文件是否存在。指向不存在文件的路径与其他路径一样会被列出。

### 打开文档

网页界面通过 `/api/documents/<路径的 base64>` 打开文档，它只提供该路径上的文件，从不提供目录：

- 账本根目录之外的路径（在本地磁盘上也包括通过符号链接到达的）会以 403 拒绝，存储拒绝读取的文件也是如此；
- 不存在的文件、目录或其他不是普通文件的条目返回 404；
- 不是有效 base64 或 UTF-8、或者包含换行或 NUL 的路径返回 400。

从远程数据源读取的文档会保存在张记账运行目录下的 `.cache/documents/` 中，之后都提供这份副本。本地磁盘上的文档每次都从磁盘读取。

### 在网页界面中

- 文档页面按从新到旧的顺序列出所有文档：`document` 指令，以及交易的 `document` 元数据，同一笔交易的按书写顺序。写在记账行上的文档既有这条记账行的账户，也有它的交易：网格显示账户，列表两者都显示。账户页面列出该账户的文档，交易的详情显示它的文档。
- 在账户页面上传的文件，保存为账本根目录下的 `attachments/<random id>/<file name>`，并添加一条日期为当前时间的 `document` 指令，写到 [`directive_output_path`](/zh-cn/reference/directives/options/#directive_output_path) 选项所选的文件中。在 Beancount 文件中，路径写成相对于该文件的形式，例如 `"../../attachments/<random id>/<file name>"`。文件名必须是不带目录的普通名称，最长 255 字节。
- 在交易上上传的文件以同样的方式保存，并在包含该交易的文件中，紧接交易首行之下添加一行 `document` 元数据。

### 在查询中

[`#documents`](/zh-cn/reference/query-language/#pricesbalancesnoteseventsdocuments-和-commodities) 中每条 `document` 指令对应一行，其后交易或记账行的每个 `document` 元数据值各对应一行。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist) | `document` 指令的账户在其日期未开立。文档仍会被列出。 |
| [`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed) | 账户已经关闭。文档仍会被列出。 |
| [`DocumentPathRelativeToRoot`](/zh-cn/reference/error-codes/#documentpathrelativetoroot) | 提示：在本地磁盘上的 Beancount 文件中，路径只有相对于账本根目录才能找到文件。文档仍会被列出，并且可以打开。 |
| [`DocumentNotFound`](/zh-cn/reference/error-codes/#documentnotfound) | 在本地磁盘上的 Beancount 文件中，相对于指令所在文件和相对于账本根目录都找不到文件。 |

## Beancount 兼容性

`document` 指令在 Beancount 中的语法相同。有两点不同：

- 张记账与 Beancount 一样，把 Beancount 文件中的相对路径解析为相对于声明该指令的文件所在的目录，并且仍能找到早期版本写成相对于账本根目录的文档（见[路径](#路径)）。在查询中，`#documents` 的 `filename` 列遵循 Beancount 的规则，`path` 列给出网页界面打开文件所用的、在账本中的路径。
- Beancount 的 `documents` 选项会按文件名在目录树中查找文档，它在张记账中没有作用。请为每个文件写一条 `document` 指令或元数据。

## 相关页面

- [文档](/zh-cn/guides/documents/)：附加收据和对账单。
- [交易](/zh-cn/reference/directives/transaction/#元数据)：交易的元数据写在哪里。
