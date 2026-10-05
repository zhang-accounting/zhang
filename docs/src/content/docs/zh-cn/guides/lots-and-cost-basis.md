---
title: 批次与成本
description: "用成本和价格跟踪投资：批次语法、记账方法、已实现收益与舍入。"
sidebar:
  order: 3
---

买入股票、基金份额或加密货币时，你持有一种商品（`AAPL`）的若干单位，它们花费了你另一种商品（`USD`）的一定金额。张记账把每次买入记为一个**批次**：单位数量、每单位成本和取得日期。卖出时，张记账从你的批次中扣除单位，因此它知道所卖部分的成本以及你实现的收益。

本指南跟随一个券商账户，经历三次买入和一次卖出。

## 准备

```zhang
option "operating_currency" "USD"

1970-01-01 commodity AAPL

2024-01-01 open Assets:Broker:Cash USD
2024-01-01 open Assets:Broker:AAPL AAPL
2024-01-01 open Income:Broker:Gains USD
2024-01-01 open Expenses:Broker:Fees USD
2024-01-01 open Equity:Opening-Balances

2024-01-02 balance Assets:Broker:Cash 10000 USD with pad Equity:Opening-Balances
```

你持有的每种商品都必须用 [`commodity`](/zh-cn/reference/directives/commodity/) 声明。主货币（这里是 `USD`）由选项声明。

## 按成本买入

把单位的成本写在花括号中：

```zhang
2024-01-10 * "买入 10 AAPL"
  Assets:Broker:AAPL 10 AAPL {185.00 USD}
  Assets:Broker:Cash -1850.00 USD

2024-03-15 * "买入 5 AAPL，共 860 USD"
  Assets:Broker:AAPL 5 AAPL {{860.00 USD}}
  Assets:Broker:Cash

2024-05-20 * "买入 5 AAPL"
  Assets:Broker:AAPL 5 AAPL {190.00 USD}
  Assets:Broker:Cash -950.00 USD
```

- `{185.00 USD}` 是一个单位的成本。`{{860.00 USD}}` 是该记账行全部单位的成本：张记账按每单位 860 / 5 = 172.00 USD 保存这个批次。复合成本 `{185.00 # 5.00 USD}` 是单位成本加上一个总额（例如佣金）：10 个单位时，张记账按每单位 185.00 + 5.00 / 10 = 185.50 USD 保存这个批次，与 Beancount 相同。
- 带成本的记账行，其权重是单位数乘以成本，即 `10 × 185.00 = 1,850.00 USD`，交易在 USD 上平衡。
- 批次在交易的日期取得。要指定另一个日期，把它写在成本之后：`{185.00 USD, 2024-01-09}`。后面还可以再加一个批次标签：`{185.00 USD, 2024-01-09, "first"}`。成本的各部分顺序任意，每个部分都可以单独出现：`{2024-01-09}` 或 `{"first"}` 也是成本。
- 买入时写空的 `{}`，会根据其他记账行的金额推导成本，并按买入日期开立新批次。例如，`3 AAPL {}` 对应 `-600 USD`，每股成本就是 `200 USD`。只能缺少一个数字：同时省略成本和现金金额，或者省略多个成本，会报告 [`TransactionCannotInferTradeAmount`](/zh-cn/reference/error-codes/#transactioncannotinfertradeamount) 并拒绝整笔交易。

账户现在持有三个批次：10 AAPL，成本 185.00 USD；5 AAPL，成本 172.00；5 AAPL，成本 190.00。

## 价格不是成本

`@` 给出一个单位的价格，`@@` 给出全部单位的价格。带价格的记账行，其权重是单位按该价格换算后的金额，但**不保存批次**：这些单位没有成本。货币兑换需要的正是这样：

```zhang
2024-02-01 * "换汇"
  Assets:Bank:USD 100 USD @ 7.20 CNY
  Assets:Bank:CNY -720.00 CNY
```

在同时带有成本的记账行上，价格只是参考信息，例如你卖出时的价格：记账行按成本计算权重。`@` 和 `@@` 都不会加入商品的价格历史。请用 [`price`](/zh-cn/reference/directives/price/) 指令记录市场价格：

```zhang
2024-08-01 price AAPL 220.00 USD
```

## 卖出并实现收益

用空成本 `{}` 卖出。张记账按账户的[记账方法](#选择记账方法)选择批次，默认为 `FIFO`：

```zhang
2024-08-01 * "卖出 12 AAPL"
  Assets:Broker:AAPL -12 AAPL {} @ 220.00 USD
  Assets:Broker:Cash 2639.00 USD
  Expenses:Broker:Fees 1.00 USD
  Income:Broker:Gains
```

- 按 `FIFO`，卖出先取最早的批次：成本 185.00 的全部 10 AAPL，以及成本 172.00 的 5 AAPL 中的 2 个。卖出单位的权重是它们的成本，即 `10 × 185.00 + 2 × 172.00 = 2,194.00 USD`。价格 `@ 220.00 USD` 不计入权重。
- 你收到的现金加上手续费共 2,640.00 USD。它与成本之差 446.00 USD 就是已实现收益。收入记账行不写金额，张记账会填上 `-446.00 USD`。你也可以自己写出收益；这时交易必须平衡。
- 账户剩下成本 172.00 USD 的 3 AAPL 和成本 190.00 USD 的 5 AAPL。

要从特定的批次卖出，用它们的成本代替 `{}`：

- `{172.00 USD}` 从成本为 172.00 USD 的批次中扣除，不论取得日期。
- `{172.00 USD, 2024-03-15}` 只从 2024 年 3 月 15 日取得的批次中扣除。
- 只写 `{2024-03-15}` 时，只从 2024 年 3 月 15 日取得的批次中扣除，不论成本。
- `{172.00 USD, "first"}`，或不写成本的 `{"first"}`（张记账也能读取旧写法 `{, "first"}`），只从标签为 `first` 的批次中扣除。买入时写上的标签，例如 `{172.00 USD, "first"}`，就是该批次的名字：只有标签不同的批次会分开保存，没写标签的买入也不会加到带标签的批次里。

Beancount 的成本合并标记 `{*}` 可以读取但不受支持：这笔卖出会报告 [`CostMergingNotSupported`](/zh-cn/reference/error-codes/#costmergingnotsupported)，并像 `{}` 一样记账。

### 卖出超过持有的数量

用 `{}` 卖出时，匹配的成本批次必须覆盖全部数量。否则张记账会报告 [`NoEnoughCommodityLot`](/zh-cn/reference/error-codes/#noenoughcommoditylot) 和 [`TransactionCannotInferTradeAmount`](/zh-cn/reference/error-codes/#transactioncannotinfertradeamount)，把整笔交易排除在账本之外，并保留之前的持仓。账户只持有不带成本的单位时，同样适用。

如果卖出时写了明确的成本，但数量超过匹配批次，张记账会报告 `NoEnoughCommodityLot`，并按该成本保留剩余数量的空头批次。之后可用带 `{}` 的正数记账行回补空头，保留原成本。

## 选择记账方法

记账方法决定用 `{}` 卖出、或用匹配多个批次的成本卖出时，从哪些批次中扣除：

| 方法 | 扣除自 |
|---|---|
| `FIFO` | 取得日期最早的批次优先。默认方法。 |
| `LIFO` | 取得日期最新的批次优先。 |
| `STRICT` | 单个匹配的批次。匹配多个批次的卖出必须把它们全部卖完，否则就有歧义。 |

取得日期相同的批次按创建顺序扣除，`LIFO` 则顺序相反。

用 `open` 上的 `booking_method` 为单个账户设置方法，或用 `default_booking_method` 选项为整个账本设置：

```zhang
option "default_booking_method" "LIFO"

2024-01-01 open Assets:Broker:Retirement AAPL
  booking_method: "STRICT"
```

在 Beancount 账本中，方法也可以写在 `open` 的商品之后：`2024-01-01 open Assets:Broker:AAPL AAPL "STRICT"`。`NONE`、`AVERAGE` 和 `AVERAGE_ONLY` 尚未实现：使用其中之一的账户或选项会得到 [`UnsupportedBookingMethod`](/zh-cn/reference/error-codes/#unsupportedbookingmethod) 错误，并按默认方法记账。见[记账方法](/zh-cn/reference/directives/account/#记账方法)和 [`default_booking_method`](/zh-cn/reference/directives/options/#default_booking_method)。

### STRICT 与有歧义的卖出

一个 `STRICT` 账户持有成本 185.00 USD 的 10 AAPL 和成本 172.00 USD 的 5 AAPL，卖出 `-6 AAPL {}` 会匹配这两个批次，却不会把它们全部卖完。张记账报告 [`AmbiguousLotMatch`](/zh-cn/reference/error-codes/#ambiguouslotmatch) 错误，并列出匹配的批次：

```text
10 AAPL {185.00 USD, 2024-01-10}, 5 AAPL {172.00 USD, 2024-03-15}
```

它仍按 `FIFO` 记录这笔卖出，因此数字仍然可用。指明你要的批次，例如 `-6 AAPL {185.00 USD}`，错误就会消失。

## 舍入

张记账使用精确的十进制数计算，保留你写下的每一位数字。金额、成本和价格，以及它们的乘积与和，永远不会被舍入。

只有除法会产生更多的位数，例如由总成本得出的单位成本：3 个单位的 `{{1000 USD}}` 是每单位 333.333… USD。当张记账填写一个缺失的金额，而其精确值超过 20 位小数时，它会按该商品的精度与交易中该商品所写的最多小数位数两者中较大的那个，用商品的舍入方式对这个金额舍入：

```zhang
2024-05-16 * "买入 3 AAPL，共 1000 USD"
  Assets:Broker:AAPL 3 AAPL {{1000 USD}}
  Assets:Broker:Cash -1000 USD

2024-05-17 * "按成本卖出 1 股"
  Assets:Broker:AAPL -1 AAPL {}
  Assets:Broker:Cash

2024-05-18 * "按成本卖出另外 2 股"
  Assets:Broker:AAPL -2 AAPL {}
  Assets:Broker:Cash
```

现金记账行分别成为 `333.33 USD` 和 `666.67 USD`：合计正好是你支付的 1,000 USD。

当一笔交易在每种商品上的权重之和按该商品的精度舍入为零时，交易就是平衡的。精度是 [`commodity`](/zh-cn/reference/directives/commodity/#精度) 指令的 `precision` 元数据，没有时为 2 位小数。因此金额更精细的商品，例如加密货币，需要设置自己的 `precision`。[余额断言](/zh-cn/guides/balances/)不做舍入：只有金额完全相等，或在你写下的 `~` 容差之内，断言才成立。

## 查看批次

- **货币**页面列出所有商品。打开其中一个，例如 `AAPL`，可以看到它的**批次**标签页：`Assets` 或 `Liabilities` 账户中持有的每个批次，及其账户、取得日期、成本和数量。
- 在[查询](/zh-cn/guides/querying/)中，`position` 显示记账行的单位及其成本，`cost()` 给出账面价值，`value()` 给出按最新 `price` 计算的市场价值：

  ```sql
  SELECT account, sum(position) AS holding, cost(sum(position)) AS book_value, value(sum(position)) AS market_value
  WHERE account ~ '^Assets:Broker:AAPL'
  GROUP BY account
  ```

  上面的卖出之后，这个查询为 `Assets:Broker:AAPL` 返回一行：持仓 `3 AAPL {172.00 USD, 2024-03-15}` 和 `5 AAPL {190.00 USD, 2024-05-20}`，账面价值 `1466.00 USD`，市场价值 `1760.00 USD`。
