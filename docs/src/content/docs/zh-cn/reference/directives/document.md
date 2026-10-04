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
| `"<Path>"` | 是 | 文件的路径，相对于账本根目录。 |
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

- **路径相对于账本根目录**，即启动 `zhang serve` 时指定的目录，不论指令写在哪个文件中。在上面的示例中，文件是账本根目录下的 `statements/2024-01.pdf` 和 `receipts/2024-02-03-cafe.jpg`。网页界面无法打开账本根目录之外的文件。
- 张记账加载账本时不检查文件是否存在。指向不存在文件的路径与其他路径一样会被列出，只是打开时会失败。
- 文档不改变任何余额。
- 文件通过账本的[数据源](/zh-cn/deployment/data-sources/local/)读取，所以存放在 S3、WebDAV 或 GitHub 上的账本，文档的用法完全相同。

### 在网页界面中

- 文档页面按从新到旧的顺序列出所有文档：`document` 指令，以及交易的 `document` 元数据。账户页面列出该账户及其子账户的 `document` 指令，交易的详情显示它的文档。
- 在账户页面上传的文件，保存为账本根目录下的 `attachments/<random id>/<file name>`，并添加一条日期为当前时间的 `document` 指令，写到 [`directive_output_path`](/zh-cn/reference/directives/options/#directive_output_path) 选项所选的文件中。
- 在交易上上传的文件以同样的方式保存，并在包含该交易的文件中，紧接交易首行之下添加一行 `document` 元数据。

### 在查询中

[`#documents`](/zh-cn/reference/query-language/#pricesbalancesnoteseventsdocuments-和-commodities) 中每条 `document` 指令对应一行，其后交易或记账行的每个 `document` 元数据值各对应一行。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist) | `document` 指令的账户在其日期未开立。文档仍会被列出。 |
| [`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed) | 账户已经关闭。文档仍会被列出。 |

## Beancount 兼容性

`document` 指令在 Beancount 中的语法相同。有两点不同：

- Beancount 把相对路径解析为相对于声明该指令的文件所在的目录。张记账的网页界面则相对于账本根目录解析。对于写在账本根目录下文件中的指令，两者一致。在查询中，`#documents` 的 `filename` 列遵循 Beancount 的规则，`path` 列遵循张记账的规则。
- Beancount 的 `documents` 选项会按文件名在目录树中查找文档，它在张记账中没有作用。请为每个文件写一条 `document` 指令或元数据。

## 相关页面

- [文档](/zh-cn/guides/documents/)：附加收据和对账单。
- [交易](/zh-cn/reference/directives/transaction/#元数据)：交易的元数据写在哪里。
