---
title: 价格
description: price 指令的参考，它记录某种商品在某一天的价格。
sidebar:
  order: 6
---

`price` 指令记录某一天一单位某种商品值多少另一种商品。张记账只用价格来估算持仓的价值：用于网页界面中的合计，以及查询中的估值函数。价格从不改变余额，也不用于检查交易。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] price <Commodity> <Number> <QuoteCommodity>
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 价格从哪一天起适用，可以附带一天中的时刻。 |
| `<Commodity>` | 是 | 被定价的商品，例如 `USD` 或 `AAPL`。 |
| `<Number>` | 是 | 一单位的价格。可以是表达式。 |
| `<QuoteCommodity>` | 是 | 价格所用的商品。 |

指令下方可以写元数据行。

## 示例

```zhang
option "operating_currency" "CNY"

2024-01-01 commodity USD
2024-01-01 commodity AAPL

2024-01-02 price USD 7.10 CNY
2024-01-02 price AAPL 185.64 USD
2024-01-03 price USD 7.12 CNY
```

## 行为

- 两种商品都必须在价格的日期或之前定义。否则这条价格会报告为 [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine)，但仍会被记录。
- 某一天的价格，是这对商品日期在当天或之前的最新一条 `price`。同一对商品在同一日期和时间有多条价格时，以账本中最后一条为准。
- 只有 `price` 指令才是价格。写在记账行上的价格（`@` 或 `@@`）只用于换算该记账行，不会添加价格。

### 在网页界面中

网页界面以[主货币](/zh-cn/reference/directives/options/#operating_currency)显示合计：账户列表上的合计，以及总览页面和统计报表页面中的数字。

- 主货币的金额按原值计入。
- 总览页面和统计报表页面的数字和图表，用查询语言的 [`convert`](/zh-cn/reference/query-language/#估值函数) 换算：取日期在该数字所对应的日期（期末，或图表中某个点的最后一天）或之前的最新价格，价格可以反向使用，按成本持有、自身没有价格的持仓通过成本货币换算。它们的[内置查询](/zh-cn/reference/builtin-queries/#报表)展示了具体做法。
- 账户列表用**该商品以主货币表示**、日期在今天或之前的最新价格换算其他商品的金额。这里只有这一对商品的价格才算数：反方向的价格不会被取倒数，所以 `price CNY 0.14 USD` 不会把 USD 换算成 CNY；换算也不会经过第三种商品。
- 任何价格都换算不了的金额不计入合计，但按商品列出的金额中仍会显示它。
- 货币页面显示每种商品以主货币表示的最新价格：即账本中最新的那一条，即使它的日期在未来。商品自己的页面列出它的所有价格。

### 在查询中

查询函数 `convert`、`value` 和 `getprice` 读取同样的 `price` 指令，但规则更宽：自身没有价格的一对商品使用反方向价格的倒数，两个方向都有报价的一对商品合并为一条价格历史，一种商品以它自己表示的价格为 1。见[估值函数](/zh-cn/reference/query-language/#估值函数)。

所以账户列表和查询对同一笔持仓的估值可能不同。只有下面这条价格时，查询以及总览和统计报表页面会把 10 EUR 换算成 80 CNY，而账户列表在 CNY 合计中不计入这些 EUR：

```zhang
option "operating_currency" "CNY"

2024-01-01 commodity EUR
2024-01-02 price CNY 0.125 EUR
```

要让账户列表为某种商品估值，请写下它以主货币表示的价格。

插件不会收到预先算好的价格。需要汇率的插件，要根据指令流中的 `price` 指令自行构建；见[汇率](/zh-cn/developers/writing-plugins/#汇率)。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine) | 两种商品之一在价格的日期未定义。 |

## Beancount 兼容性

语法与 Beancount 相同。在 Beancount 文件中，一天中的时刻写成 `time: "HH:MM:SS"` 元数据。Beancount 的 `implicit_prices` 插件会把记账行上的价格变成价格条目，它在张记账中不会运行；[查询函数](/zh-cn/reference/query-language/#估值函数)只使用 `price` 指令。

## 相关页面

- [商品](/zh-cn/reference/directives/commodity/)：定义价格中提到的商品。
- [查询](/zh-cn/guides/querying/)：用查询为持仓估值。
