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

- 在张记账账本中，路径相对于账本根目录，即 `zhang serve` 所服务的目录。在 Beancount 账本中，它与 Beancount 的读法一样，相对于该指令所在的文件。请把文档放在账本根目录内：张记账不会打开根目录之外的文件，通过符号链接也不行。见[路径](/zh-cn/reference/directives/document/#路径)。
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

`document` 元数据的路径在两种账本中都相对于账本根目录。打开一个文件不存在的文档时，会提示文件不存在。在本地磁盘上的 Beancount 账本中，文件不存在的 `document` 指令还会报告为 [`DocumentNotFound`](/zh-cn/reference/error-codes/#documentnotfound)。

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

  在 Beancount 账本中，路径写成相对于该文件的形式，例如 `data/2024/04.bean` 中的 `"../../attachments/78e12a54-d9e5-4de5-9de3-f140308e1c79/scan.pdf"`，这样 Beancount 也能找到文件。

- 对于交易，它在交易首行的正下方添加一行 `document:`，写入该交易所在的文件。

文件名必须是不带目录的普通名称，最长 255 字节。`attachments/` 目录是固定的，没有选项可以修改。账本存放在 [S3、WebDAV 或 GitHub](/zh-cn/deployment/data-sources/s3/) 上时，上传的文件写入该存储中的同一位置，文档也从那里读取。

## 查看文档

- **文档**页面按从新到旧的顺序列出所有文档，可以用网格或列表显示，并标明每个文档所属的账户或交易。图片可以预览，其他文件会在新标签页中打开。
- 账户的**附件**标签页列出该账户的文档。
- 流水页面中交易的预览显示它的文档，流水中也会标记出带有文档的交易。

对于存放在远程数据源上的账本，张记账会把显示过的每个文档复制一份，保存在它运行目录下的 `.cache/documents/` 中，之后都提供这份副本。如果你替换了文件但保留了文件名，请删除副本，或者给新文件换个名字。本地磁盘上的文档每次都从磁盘读取。早期版本保存在 `.cache/data/` 中的副本不再使用，可以删除该目录。

文档也可以在[查询](/zh-cn/guides/querying/)中使用，即 `#documents` 表。
