---
title: 商品
description: commodity 指令的参考，以及张记账从中读取的元数据，例如精度、舍入方式、前缀和后缀。
sidebar:
  order: 3
---

`commodity` 指令定义一种商品：货币、股票、加密货币，或者其他任何你要计数的东西。它的元数据决定张记账如何显示和舍入这种商品的金额。使用某种商品的交易、价格或 `open`，都需要这种商品先被定义。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] commodity <Name>
  [<key>: <value>]
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 商品从哪一天起有定义。 |
| `<Name>` | 是 | 以一个 ASCII 字母开头，后面跟字母、数字、`.`、`_`、`-` 或 `'`，例如 `USD`、`AAPL` 或 `VBMPX`。 |
| `<key>: <value>` | 否 | 元数据行，见[元数据](#元数据)。 |

## 示例

```zhang
1970-01-01 commodity CNY
  prefix: "¥"
  group: "Fiat currencies"

1970-01-01 commodity JPY
  precision: 0
  prefix: "¥"
  group: "Fiat currencies"

1970-01-01 commodity BTC
  precision: 8
  suffix: " BTC"
  rounding: "RoundUp"
  group: "Crypto currencies"
```

## 元数据

| 键 | 值 | 默认值 |
|---|---|---|
| [`precision`](#精度) | 小数位数，一个整数 | [`default_commodity_precision`](/zh-cn/reference/directives/options/#default_commodity_precision) 选项，否则为 `2` |
| [`rounding`](#舍入) | `RoundDown` 或 `RoundUp` | [`default_rounding`](/zh-cn/reference/directives/options/#default_rounding) 选项，否则为 `RoundDown` |
| `prefix` | 网页界面中显示在数字前面的文本，例如 `$` | 无 |
| `suffix` | 网页界面中显示在数字后面的文本 | 无 |
| `group` | 货币页面把这种商品列在哪个标题下 | 无：商品列在默认分组中 |

没有 `prefix` 和 `suffix` 时，网页界面在数字后面显示商品的名称。其他元数据会被保留；查询从 `#commodities` 中读取它们。

### 精度

商品的小数位数。它用于：

- 网页界面，按这个位数显示该商品的金额；
- 检查交易是否配平：交易在该商品上的权重之和，按商品的舍入方式舍入到这个精度后，必须为零。精度为 2 时，相差 `0.004` 的交易可以配平，相差 `0.006` 的不能。见[交易](/zh-cn/reference/directives/transaction/#交易如何配平)；
- 当除法让张记账为记账行推断出的金额超过 20 位小数时（例如把总成本分摊到 3 个单位上），对这个金额进行舍入。

不是整数的值会被忽略，此时使用默认值。

### 舍入

在上述情形中，该商品的金额如何舍入到它的精度：

- `RoundDown`：舍去的第一位小数是 5 时向下舍入，所以精度为 2 时 `0.005` 舍入为 `0.00`。
- `RoundUp`：舍去的第一位小数是 5 时向上舍入，所以精度为 2 时 `0.005` 舍入为 `0.01`。

两种模式下，其他数字都舍入到最接近的值。这个值区分大小写：其他任何值都会让账本无法加载，并给出消息 `option value is invalid`。网页界面会自行对显示的金额四舍五入。

## 行为

- 商品从它的日期起有定义。日期早于定义的交易、`price` 或 `open` 会报告 [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine)。同一日期内，`open` 和 `commodity` 保持文件中的顺序：把 `commodity` 写在列出它的 `open` 上方。
- 对于交易，张记账检查它配平时所用的商品：普通记账行的数量单位、带 `@` 的记账行的价格商品、带成本的记账行的成本商品。
- [主货币](/zh-cn/reference/directives/options/#operating_currency)由它的选项定义，默认精度为 2。为它写一条 `commodity` 指令会替换这个定义，例如为了给它加上前缀。
- 同一名称的第二条 `commodity` 指令会完全替换第一条：它没有写出的元数据恢复为默认值。
- 网页界面的货币页面按分组列出每种商品，以及它在 `Assets` 和 `Liabilities` 账户中的持有总量和以主货币表示的最新价格。

## 错误

`commodity` 指令不会产生账本错误。无效的 `rounding` 会让账本无法加载。

## Beancount 兼容性

这个指令在 Beancount 中的语法相同。张记账读取上面列出的元数据；为其他工具编写的元数据会被保留，在张记账中没有作用。张记账还接受 Beancount 拒绝的名称，例如含有小写字母的名称。

## 相关页面

- [价格](/zh-cn/reference/directives/price/)：商品之间的价格。
- [选项](/zh-cn/reference/directives/options/)：精度和舍入方式的默认值。
