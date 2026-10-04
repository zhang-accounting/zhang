---
title: 文档
description: 在账本文件中，或通过网页界面上传，为账户和交易附加收据和对账单。
sidebar:
  order: 5
---

把凭证放在数字旁边：为账户附加银行对账单，为交易附加收据。文件和账本保存在一起，网页界面会在它们所属的位置显示它们。

## 为账户附加对账单

把文件放进账本目录，再用一条 [`document`](/zh-cn/reference/directives/document/) 指令指向它：

```zhang
2024-02-01 document Assets:Bank:Checking "statements/2024/2024-01-bank.pdf" #statement
```

- 路径相对于账本根目录，即 `zhang serve` 所服务的目录。请把文档放在根目录内：张记账无法打开根目录之外的文件。
- 日期是文档的日期，例如对账单的出具日期。路径之后可以跟标签和链接。
- 该账户在这个日期必须已经开设。

## 为交易附加收据

为交易添加 `document` 元数据，每个文件一条：

```zhang
2024-03-09 * "酒店" "杭州两晚"
  document: "receipts/hotel-0309.jpg"
  Liabilities:CreditCard -840 CNY
  Expenses:Travel
```

写在某条记账行上的 `document` 元数据也属于这笔交易。

张记账不检查文件是否存在。指向不存在文件的路径会显示为一个空文档。

## 在网页界面中上传

可以在三个地方上传文件：

- **文档**页面：选择**上传**，选择账户和文件；
- 账户页面的**附件**标签页：把文件拖到上传区域；
- 流水页面中交易的预览：把文件拖到其文档下方的上传区域。

张记账把每个上传的文件保存为账本根目录下的 `attachments/<random id>/<file name>`，并在账本中记录它：

- 对于账户，它添加一条日期为当前时刻的 `document` 指令，写入 [`directive_output_path`](/zh-cn/guides/recording-transactions/#新条目写入的位置) 选项指定的文件：

  ```zhang
  2024-04-03 21:55:03 document Assets:Bank:Checking "attachments/78e12a54-d9e5-4de5-9de3-f140308e1c79/scan.pdf"
  ```

- 对于交易，它在交易首行的正下方添加一行 `document:`，写入该交易所在的文件。

`attachments/` 目录是固定的，没有选项可以修改。账本存放在 [S3、WebDAV 或 GitHub](/zh-cn/deployment/data-sources/s3/) 上时，上传的文件写入该存储中的同一位置，文档也从那里读取。

## 查看文档

- **文档**页面按从新到旧的顺序列出所有文档，可以用网格或列表显示。网格显示文档的账户，没有账户时显示它的交易；列表两者都显示。写在记账行上的文档既有这条记账行的账户，也有它的交易。同一笔交易的文档按书写顺序排列。图片可以预览，其他文件会在新标签页中打开。
- 账户的**附件**标签页列出该账户的文档。
- 流水页面中交易的预览显示它的文档，流水中也会标记出带有文档的交易。

张记账会把显示过的每个文档复制一份，保存在它运行目录下的 `.cache/data/` 中，之后都提供这份副本。如果你替换了文件但保留了文件名，请删除副本，或者给新文件换个名字。

文档也可以在[查询](/zh-cn/guides/querying/)中使用，即 `#documents` 表。
