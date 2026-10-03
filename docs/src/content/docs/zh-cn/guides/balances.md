---
title: 余额
description: 从日常记账的角度，用余额断言检查账户余额，用补齐填上差额。
sidebar:
  order: 2
---

余额断言声明某个账户在某一时刻持有多少，就像银行对账单或你的钱包所显示的那样。张记账会用该账户的记账行来核对它。定期断言余额，可以在打错、漏记和重复记录的交易还容易找到时就发现它们。

补齐是另一半：它填上一笔你无法或不想详细记录的金额，例如账户在你开始记账之前已有的钱。

两者的确切语法见 [`balance`](/zh-cn/reference/directives/balance/)，补齐的细节见[用 `with pad` 补齐](/zh-cn/reference/directives/balance/#用-with-pad-补齐)。

## 断言余额

你一月份的银行对账单期末余额为 16,643.60 CNY：

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Income:Salary CNY
2024-01-01 open Expenses:Food CNY
2024-01-01 open Equity:Opening-Balances CNY

; 账本开始之前账户已有的 5,000 CNY，见下文
2024-01-01 balance Assets:Bank:Checking 5000 CNY with pad Equity:Opening-Balances

2024-01-05 * "ACME Corp" "一月工资"
  Assets:Bank:Checking 12000 CNY
  Income:Salary

2024-01-20 * "超市"
  Assets:Bank:Checking -356.40 CNY
  Expenses:Food

2024-02-01 balance Assets:Bank:Checking 16643.60 CNY
```

- 断言在其日期的开始时检查：它计入日期在它之前的所有条目，不计入当天的任何条目。要核对截至 1 月 31 日的对账单，请把断言的日期写为 2 月 1 日。
- 带上时间，例如 `2024-01-31 23:59:59 balance …`，它也会计入当天更早的条目。没有时间的条目视为 `00:00:00`，断言排在日期和时间相同的其他条目之前。
- 账户的余额包含它的子账户：`balance Assets:Bank …` 会把 `Assets:Bank`、`Assets:Bank:Checking` 以及 `Assets:Bank` 下的其他所有账户合在一起检查。
- 一条断言检查一种商品。持有多种商品的账户，每种商品写一行。账户未持有的商品按零计算。

### 精确匹配与容差

只有余额与金额完全相等时，断言才成立。余额为 `16643.60 CNY` 时，`16643.604 CNY` 不成立；余额为 `16643.604` 时，`16643.6` 也不成立。

如果你的数据来源会做舍入，例如某个应用把基金显示到两位小数，而份额的小数位更多，请在 `~` 之后写上你接受的容差：

```zhang
2024-02-01 balance Assets:Bank:Checking 16643.60 ~ 0.01 CNY
```

当余额与 16,643.60 CNY 相差不超过 0.01 CNY 时，这条断言成立。张记账从不根据你写的小数位数推导容差，也没有任何选项能放宽断言：`default_balance_tolerance_precision` 虽然名字如此，也不能。

## 从已有余额开始

开始记账时，你的银行账户里已经有钱了。不必记录它的全部历史，把它补齐到当时的余额即可：

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Equity:Opening-Balances CNY

2024-01-01 balance Assets:Bank:Checking 5000 CNY with pad Equity:Opening-Balances
```

`with pad Equity:Opening-Balances` 让张记账添加一笔交易，使账户达到 5,000 CNY，差额取自 `Equity:Opening-Balances`：

- 补齐交易的标记是 `P`，收款方是 `Balance Pad`，摘要是 `pad Assets:Bank:Checking to Equity:Opening-Balances`。它的日期是断言的日期，并排在当天其他条目之前，因此账户在当天开始时持有 5,000 CNY。流水页面把它列为**填充**。
- 它的金额根据记账行在那一刻给出的余额计算。如果账户已经持有该金额，张记账不添加任何交易。
- 然后，断言像其他断言一样，按补齐后的余额检查。
- 补齐不限于期初余额。它也可以填上一个你永远不会去还原的缺口，例如一个月零星现金开销之后的钱包：

```zhang
2024-01-01 balance Assets:Cash 200 CNY with pad Equity:Opening-Balances

2024-01-10 * "面包店"
  Assets:Cash -18 CNY
  Expenses:Food

; 点数后有 150 CNY：另外 32 CNY 花在了不值得记录的地方
2024-02-01 balance Assets:Cash 150 CNY with pad Expenses:Misc
```

这里张记账在 2 月 1 日把 32 CNY 从 `Assets:Cash` 转到 `Expenses:Misc`。

### Beancount 的 `pad`

Beancount 账本把补齐和断言写成两条指令：

```beancount title="main.bean"
2024-01-01 pad Assets:Bank:Checking Equity:Opening-Balances
2024-01-02 balance Assets:Bank:Checking 1000.00 USD
```

张记账像 Beancount 一样把它们配对：一条 `pad` 服务于其账户在每种商品上的下一条 `balance`，直到该账户的下一条 `pad` 为止。两者必须在同一个文件中。与 `pad` 同一天的 `balance` 不会被补齐，后面没有 `balance` 的 `pad` 不起任何作用。

每一对的效果都与 `balance … with pad` 相同，因此补齐交易的日期是 `balance` 的日期，这里是 1 月 2 日，而 Beancount 把它记在 `pad` 的日期。1 月 2 日及之后的余额是一样的。`pad` 指令只存在于 Beancount 文件中；在张记账文件中，请写 `with pad`。

## 断言失败时

断言失败不会改变你的账目：账户保留记账行给出的余额，之后的断言和补齐也从这个余额开始计算。张记账只是报告它：

- 总览页面上的账本错误列表显示一个 [`AccountBalanceCheckError`](/zh-cn/reference/error-codes/#accountbalancecheckerror)，并给出账户名。
- 在流水页面中，这条断言标记为**断言失败**。它的预览显示断言余额、累计余额、差额和容差。
- 在账户页面中，**记录**标签页把这条断言显示为一行，所断言的金额显示在累计余额旁边。

查找原因：

1. 看差额。它往往等于某一笔交易：漏记的、记了两次的，或者正负号写反的（差额是其金额的两倍）。
2. 列出该账户的记账行及其累计余额，与对账单逐行比对：

   ```sql
   JOURNAL 'Assets:Bank:Checking' FROM date >= 2024-01-01 AND date < 2024-02-01
   ```

3. 用更多断言缩小时间范围，例如每张对账单或每周一条。第一条失败的断言告诉你该从哪里找。
4. 在[查询](/zh-cn/guides/querying/)页面中列出所有失败的断言及其差额，即记账行给出的余额减去断言的金额：

   ```sql
   SELECT date, account, amount, discrepancy FROM #balances WHERE discrepancy IS NOT NULL
   ```

找到并修正这些交易后，断言就会重新成立。如果你认为差额不值得追查，可以把断言改成补齐到某个支出账户的 `balance … with pad`，有意地记下这笔差额。

## 在网页界面中断言余额

- 在账户页面中，**余额断言**标签页列出账户持有的商品。输入其中一种的实际余额，然后选择**断言余额**。要补齐差额，在**填充来源**下选择一个账户，然后选择**填充并断言**。
- **工具** → **批量对账**可以一次为多个账户完成同样的操作。留空的行会被跳过。

网页界面用当前的日期和时间标注这些断言，因此它们计入截至此刻记录的所有内容，包括今天的条目。它们会写入 [`directive_output_path`](/zh-cn/guides/recording-transactions/#新条目写入的位置) 选项指定的文件。

## 插件、补齐与断言

张记账按以下顺序处理账本：先按声明顺序运行[插件](/zh-cn/guides/plugins/)，再检查每个账户是否已开设，然后补齐，最后检查断言。因此插件添加的交易也计入补齐所要达到的余额和断言检查的余额。插件能看到你的 `balance … with pad` 指令，但看不到补齐交易，因为插件运行时它们还不存在。
