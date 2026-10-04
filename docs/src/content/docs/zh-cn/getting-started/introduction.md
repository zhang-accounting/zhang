---
title: 简介
description: 张记账是什么，它所基于的纯文本记账的核心概念，以及这份文档的组织方式。
sidebar:
  order: 1
---

张记账（Zhang，即“账”的拼音）是一款纯文本的复式记账工具。你把账记在自己拥有的文本文件里，张记账在这些文件之上提供网页界面。它直接读取 [Beancount](https://beancount.github.io/) 账本，所以你可以把它指向现有的 Beancount 账本，无需做任何转换。

张记账适合这样的人：想要精确地记录个人或家庭的财务，希望数据保存为可以阅读、比较差异和备份的格式，同时又想要仪表盘、报表，以及在浏览器里记一笔消费的表单。它由你自己运行，可以在自己的电脑上，也可以在服务器上。

## 核心概念

**账本文件。** 账本由一个或多个文本文件组成，文件内容是一条条*指令*：以日期开头、说明发生了什么的条目，例如开设一个账户或记录一笔交易。张记账从主文件（默认为 `main.zhang`）开始，顺着其中的 [`include`](/zh-cn/reference/directives/include/) 指令读取其他文件。指令的先后顺序无关紧要，张记账会按日期排序。以 `.bean`、`.beancount` 或 `.bc` 结尾的文件按 Beancount 格式读取。其他文件使用张记账格式，它与 Beancount 的格式很接近，并增加了几种指令，例如[预算](/zh-cn/reference/directives/budget/)。

**账户。** 金额存放在账户中，账户名是分层的，例如 `Assets:Bank:Checking`。名称的第一部分是五种账户类型之一：

- `Assets`：你拥有的东西，例如现金、银行账户和投资。
- `Liabilities`：你欠下的钱，例如信用卡和贷款。
- `Equity`：期初余额的来源。
- `Income`：钱的来处，例如工资或利息。
- `Expenses`：钱的去处，例如餐饮或房租。

账户必须先[开设](/zh-cn/reference/directives/account/)才能使用，之后也可以关闭。

**货币与商品。** 每个金额都有一种商品：可以是 `USD` 这样的货币，也可以是你要计数的任何东西，例如股票或航空里程。用 [`commodity`](/zh-cn/reference/directives/commodity/) 指令声明它们，用 [`price`](/zh-cn/reference/directives/price/) 记录它们随时间变化的价值。

**交易与记账行。** [交易](/zh-cn/reference/directives/transaction/)在账户之间转移金额。交易中的每一个账户行都是一条*记账行*。一笔交易的所有记账行之和必须为零，这就是复式记账的含义：钱总是从某处来，到某处去。其中一条记账行可以省略金额，由张记账自动补上。

**断言。** [`balance`](/zh-cn/reference/directives/balance/) 指令声明某个账户在某一天持有多少，例如银行对账单上印的余额。张记账会检查它，账目不一致时报告错误。除非你明确给出容差，断言必须精确相等。

**网页界面读取的是文件。** 张记账把账本加载到内存中，并据此提供网页界面。你在界面中记录的内容会作为普通指令写回文件；你自己修改文件时，张记账会重新加载账本（何时会重新加载，见[本地文件系统](/zh-cn/deployment/data-sources/local/)）。账目中的问题，例如不平衡的交易或不成立的断言，会出现在网页界面的错误列表中。

## 文档的组织方式

- [入门](/zh-cn/getting-started/installation/)：安装张记账，编写[你的第一个账本](/zh-cn/getting-started/first-ledger/)，或者[导入你的 Beancount 账本](/zh-cn/getting-started/from-beancount/)。
- [指南](/zh-cn/guides/recording-transactions/)：日常任务，例如记录交易、核对余额、跟踪投资、预算、文档、查询和插件。
- [部署](/zh-cn/deployment/data-sources/local/)：账本存放在哪里（本地磁盘、S3、WebDAV 或 GitHub），以及登录和升级。
- [参考](/zh-cn/reference/directives/options/)：所有指令和选项、[查询语言](/zh-cn/reference/query-language/)以及[错误码](/zh-cn/reference/error-codes/)。
- [开发者](/zh-cn/developers/writing-plugins/)：编写插件，以及熟悉源代码。
