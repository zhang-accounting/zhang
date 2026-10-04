---
title: 你的第一个账本
description: 编写一个最小的账本，用张记账打开它，浏览网页界面，并在浏览器中记录一笔交易。
sidebar:
  order: 3
---

本页带你完成一个小账本：一个有期初余额的银行账户、一笔工资、几笔支出和一次余额核对。你需要先安装好张记账，见[安装](/zh-cn/getting-started/installation/)。

## 编写账本

新建一个文件夹，例如 `~/ledger`，并在其中创建名为 `main.zhang` 的文件：

```zhang
option "title" "我的账本"
option "operating_currency" "USD"

1970-01-01 commodity USD
  precision: 2

2024-01-01 open Assets:Bank:Checking USD
2024-01-01 open Liabilities:CreditCard USD
2024-01-01 open Income:Salary USD
2024-01-01 open Expenses:Food USD
2024-01-01 open Expenses:Rent USD
2024-01-01 open Equity:Opening-Balances USD

2024-01-01 * "期初余额"
  Assets:Bank:Checking     2500.00 USD
  Equity:Opening-Balances

2024-01-05 * "ACME Corp" "一月工资"
  Assets:Bank:Checking     3000.00 USD
  Income:Salary

2024-01-06 * "房东" "一月房租"
  Assets:Bank:Checking    -1200.00 USD
  Expenses:Rent

2024-01-08 * "街角咖啡馆" "午餐"
  Liabilities:CreditCard    -12.50 USD
  Expenses:Food

2024-01-31 balance Assets:Bank:Checking 4300.00 USD
```

各部分的作用：

- [选项](/zh-cn/reference/directives/options/)为账本命名（显示在网页界面中），并设置汇总和报表使用的货币。
- [`commodity`](/zh-cn/reference/directives/commodity/) 指令声明 `USD`，显示时保留两位小数。
- [`open`](/zh-cn/reference/directives/account/) 指令创建账户。每个账户名后面的 `USD` 是可选的，用来注明该账户持有的货币。
- 第一笔交易把原本就在银行里的钱记入账本。这笔钱来自 `Equity:Opening-Balances`，这是存放期初余额的常用账户。
- 其他[交易](/zh-cn/reference/directives/transaction/)记录了一笔工资、房租和一顿用信用卡付款的午餐。这里的每笔交易都省略了最后一条记账行的金额：张记账会把它补上，使所有记账行之和为零。
- [`balance`](/zh-cn/reference/directives/balance/) 指令断言银行账户在 1 月 31 日恰好持有 4300.00 USD：2500 + 3000 − 1200。

## 启动张记账

```shell
zhang serve ~/ledger
```

或者使用 Docker：

```shell
docker run --name zhang -v "$HOME/ledger:/data" -p "8000:8000" kilerd/zhang:latest
```

在浏览器中打开 `http://localhost:8000`。如果命令立刻退出，说明账本无法加载，例如语法中有笔误：用 `RUST_LOG=info zhang serve ~/ledger` 再运行一次，就能看到出错的文件和行。

## 网页界面导览

网页界面包含以下页面：

- **总览**：最近 30 天的摘要、净资产和现金流图表，以及账本的健康状况（加载时发现的错误）。
- **流水**：账本中的交易和余额断言，最新的在前，支持搜索以及按标签和链接筛选。选中一条记录可以查看它的记账行、元数据和文档，或者编辑交易。
- **统计报表**：你所选时间段内的收入、支出和净资产，收入和支出按账户细分。
- **资产负债**（小屏幕上为**账户**）：每个账户及其余额。打开一个账户可以查看它及其子账户的记账行、文档和余额走势，并记录一次余额断言。
- **预算**：按月显示你的[预算](/zh-cn/guides/budgets/)：分配了多少、花了多少、还剩多少。
- **货币**：账本中的货币和资产，以及持仓、批次和价格历史。
- **文档**：关联到账户和交易的收据和账单。你可以上传新文档，见[文档](/zh-cn/guides/documents/)。
- **编辑**：在浏览器中编辑账本文件。保存文件会重新加载账本。
- **查询**：用[兼容 BQL 的语言](/zh-cn/reference/query-language/)执行查询，见[查询](/zh-cn/guides/querying/)。
- **工具**：实用工具，例如一次性为多个账户对账或补齐余额。
- **设置**：语言和主题，账本的标题、主货币和选项，张记账的版本，已加载的插件，API 文档的链接，以及启用了[通行密钥登录](/zh-cn/deployment/authentication/#通行密钥)时你的通行密钥。

导航栏中还有一个**新建交易**按钮、一个重新加载账本的按钮，以及账户列表。

## 在网页界面中记录交易

点击**新建交易**并填写表单：日期（默认为今天）、收款方（例如 `街角咖啡馆`）、摘要（例如 `咖啡`）以及记账行。对于一杯用信用卡付款的咖啡，填写 `Liabilities:CreditCard`，金额为 `-4.50 USD`，再填写 `Expenses:Food`，不填金额。点击**保存**。

张记账把这笔交易写入以其月份命名的文件，日期在 2024 年 2 月时为 `data/2024/02.zhang`，并在第一次写入时向 `main.zhang` 添加一条引入该文件的 `include`：

```zhang
2024-02-03 12:30:00 * "街角咖啡馆" "咖啡"
  Liabilities:CreditCard -4.50 USD
  Expenses:Food
```

日期会带上当天的时间，使用账本的时区。新文件的路径由 [`directive_output_path`](/zh-cn/reference/directives/options/) 选项决定，见[记录交易](/zh-cn/guides/recording-transactions/)。账本随后重新加载，这笔交易出现在**流水**中。

## 错误显示在哪里

把余额断言改为 `4200.00 USD` 并保存文件。张记账会重新加载账本（如果没有，请使用重新加载按钮，并参阅[何时重新加载文件](/zh-cn/deployment/data-sources/local/#张记账何时重新加载)），随后出现一个错误：导航栏中**总览**旁边会显示错误数量，总览页面会列出这些错误。选中一个错误，可以看到它来自哪个文件以及对应条目的源文本。这里是一个 [`AccountBalanceCheckError`](/zh-cn/reference/error-codes/#accountbalancecheckerror)：账户持有 4300.00 USD，而不是 4200.00 USD。[错误码](/zh-cn/reference/error-codes/)解释了每种错误及其修复方法。

这些错误不会让张记账停止：账本的其余部分照常显示。语法错误则不同，因为张记账根本无法读取该文件：

- 启动时，`zhang serve` 会报错退出。
- 服务器运行期间，重新加载会失败，网页界面继续显示修改之前的账本，错误列表中也没有任何错误。原因只会写入日志（使用 Docker 时用 `docker logs zhang` 查看）。修正文件后再保存一次即可。

## 下一步

- [记录交易](/zh-cn/guides/recording-transactions/)介绍标签、链接、元数据，以及新条目写入的位置。
- [余额](/zh-cn/guides/balances/)解释余额断言和补齐。
- [本地文件系统](/zh-cn/deployment/data-sources/local/)解释张记账何时重新加载你修改过的文件。
