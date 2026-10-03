---
title: 选项
description: 张记账读取的每个选项，及其可取的值、默认值和作用。
sidebar:
  order: 1
---

`option` 指令设置作用于整个账本的设置，例如主货币或时区。本页列出张记账读取的所有选项。

## 语法

```text
option "<Key>" "<Value>"
```

| 部分 | 必填 | 说明 |
|---|---|---|
| `"<Key>"` | 是 | 选项的名称。 |
| `"<Value>"` | 是 | 选项的值，总是写成字符串。 |

`option` 没有日期，也没有元数据。它可以写在账本的任何文件、任何位置；惯例是写在主文件的开头。

## 示例

```zhang
option "title" "Family Ledger"
option "operating_currency" "USD"
option "timezone" "America/New_York"
option "default_booking_method" "FIFO"
option "directive_output_path" "data/{{year}}/{{month_str}}.{{ext}}"
```

## 行为

- 同一个键设置多次时，以最后读到的值为准。张记账先读主文件，再读它[包含](/zh-cn/reference/directives/include/#每个文件只读取一次)的文件。`operating_currency` 被设置两次时还会报告错误。
- 张记账不认识的选项会被保留：网页界面的设置页面和 HTTP API（`GET /api/options`）会列出它，[插件](/zh-cn/reference/directives/plugin/#插件收到的设置)会把它作为一项设置收到。除此之外它没有任何作用。
- 无论写在哪里，选项都在所有带日期的指令之前生效。

## 选项

| 键 | 值 | 默认值 |
|---|---|---|
| [`title`](#title) | 任意文本 | 无 |
| [`operating_currency`](#operating_currency) | 一种货币 | `CNY` |
| [`timezone`](#timezone) | IANA 时区名称 | 系统的时区 |
| [`default_booking_method`](#default_booking_method) | `STRICT`、`FIFO` 或 `LIFO` | `FIFO` |
| [`default_commodity_precision`](#default_commodity_precision) | 整数 | `2` |
| [`default_rounding`](#default_rounding) | `RoundDown` 或 `RoundUp` | `RoundDown` |
| [`default_balance_tolerance_precision`](#default_balance_tolerance_precision) | 整数 | `2` |
| [`directive_output_path`](#directive_output_path) | 路径模板 | `data/{{year}}/{{month_str}}.{{ext}}` |
| [`features.plugin`](#featuresplugin) | `true` 或 `false` | `false` |
| [`account_previous_balances` 及另外五个选项](#查询中的会计期间) | 账户名、一种货币 | 与 Beancount 相同 |

### `title`

账本的名称。网页界面在侧边栏和浏览器标签页的标题中显示它。

### `operating_currency`

网页界面显示合计时所用的货币：账户的价值，以及总览页面和统计报表页面中的数字。其他货币的金额会用[价格](/zh-cn/reference/directives/price/#在网页界面中)换算成这种货币。

- 这个选项本身就定义了该货币，所以它不需要 `commodity` 指令。它的精度是在此选项之前设置的 [`default_balance_tolerance_precision`](#default_balance_tolerance_precision) 的值，舍入方式是在此选项之前设置的 [`default_rounding`](#default_rounding) 的值。为它写一条 [`commodity`](/zh-cn/reference/directives/commodity/) 指令会替换这个定义。
- 张记账只支持一种主货币。第二次设置这个选项，会在该选项上报告 [`MultipleOperatingCurrencyDetect`](/zh-cn/reference/error-codes/#multipleoperatingcurrencydetect) 错误；张记账使用最后一个值，并把每个值都定义为货币。
- 没有这个选项时，主货币为 `CNY`，并且 `CNY` 已被定义。

### `timezone`

账本的时区，即 IANA 名称，例如 `Asia/Shanghai`、`Europe/London` 或 `UTC`。

- 账本中的日期和时间按这个时区读取：没有时间的日期表示该时区的午夜。
- 在网页界面中创建的条目，以该时区的当前时间作为日期。
- 插件按这个时区读取当前时间；如果账本依赖日期，`zhang serve` 会在该时区的午夜重新加载它。

没有这个选项时，张记账使用系统的时区；无法检测系统时区时使用 `Asia/Hong_Kong`。不是有效时区的名称会被忽略，服务器日志中会留下一条消息，此时使用系统的时区。

### `default_booking_method`

`open` 没有 `booking_method` 元数据的账户所用的记账方法：决定 `-5 AAPL {}` 这样的减仓从哪个批次中扣除数量。可取的值为 `STRICT`、`FIFO` 和 `LIFO`；各自的作用见[记账方法](/zh-cn/reference/directives/account/#记账方法)。

`AVERAGE`、`AVERAGE_ONLY` 和 `NONE` 尚未实现：它们会报告 [`UnsupportedBookingMethod`](/zh-cn/reference/error-codes/#unsupportedbookingmethod) 错误。其他任何值会报告 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta) 错误。这两种情况下，默认值都保持原样：除非之前设置过，否则为 `FIFO`。

### `default_commodity_precision`

没有有效 `precision` 元数据的[商品](/zh-cn/reference/directives/commodity/#精度)所用的精度：网页界面显示几位小数，以及交易配平时所用的小数位数。它不适用于 `operating_currency` 定义的货币。

### `default_rounding`

没有 `rounding` 元数据的[商品](/zh-cn/reference/directives/commodity/#舍入)所用的舍入方式；如果它设置在 `operating_currency` 之前，也是该选项所定义货币的舍入方式。

- `RoundDown`：舍去的第一位小数是 5 时向下舍入，所以精度为 2 时 `0.005` 舍入为 `0.00`。
- `RoundUp`：舍去的第一位小数是 5 时向上舍入，所以精度为 2 时 `0.005` 舍入为 `0.01`。

两种模式下，其他数字都舍入到最接近的值。这个值区分大小写：其他任何值（例如 `round_down`）都会让账本无法加载，并给出消息 `option value is invalid`。

### `default_balance_tolerance_precision`

这个选项虽然叫这个名字，却不给余额断言任何容差：除非用 `~` 写明容差，否则断言必须精确相等（见[余额](/zh-cn/reference/directives/balance/)）。它只设置 [`operating_currency`](#operating_currency) 所定义货币的精度，而且只在设置于该选项之前时生效。不是整数的值会被忽略。要设置主货币的精度，最好使用带 `precision` 元数据的 `commodity` 指令。

### `directive_output_path`

网页界面把它创建的条目（交易、余额断言、文档等）写到哪里。值是相对于账本根目录的路径，写成 [Jinja](https://jinja.palletsprojects.com/) 模板，可以使用以下占位符，它们取自条目的日期：

| 占位符 | 值 |
|---|---|
| `{{year}}` | 年份，例如 `2024` |
| `{{month}}` | 月份，不补零：`1` 到 `12` |
| `{{month_str}}` | 两位数的月份：`01` 到 `12` |
| `{{day}}` | 日，不补零 |
| `{{day_str}}` | 两位数的日 |
| `{{type}}` | 条目的种类，例如 `Transaction`、`BalanceCheck`、`BalancePad` 或 `Document` |
| `{{ext}}` | 主文件的扩展名，例如 `zhang` 或 `bean`，因此新条目会以账本自己的格式写入 |

使用默认值时，`main.zhang` 账本把日期在 2024 年 1 月的条目写入 `data/2024/01.zhang`，`main.bean` 账本则写入 `data/2024/01.bean`。尚不存在的文件会被创建，并在主文件末尾追加一条包含它的 `include`。不是有效模板的值会让账本无法加载。

```zhang
; 每月一个文件（默认）
option "directive_output_path" "data/{{year}}/{{month_str}}.{{ext}}"

; 每天一个文件
option "directive_output_path" "data/{{year}}/{{month_str}}/{{day_str}}.{{ext}}"

; 每年每种条目一个文件
option "directive_output_path" "data/{{year}}/{{type}}.{{ext}}"

; 只用一个文件
option "directive_output_path" "data/ledger.{{ext}}"
```

### `features.plugin`

值为 `"true"`（不区分大小写）时启用[插件](/zh-cn/reference/directives/plugin/)。其他任何值都会关闭插件。`features.plugins` 是同一个选项的另一个名字；两者之中以最后读到的为准。没有这个选项时，`plugin` 指令会被忽略。

### 查询中的会计期间

[查询](/zh-cn/reference/query-language/#会计期间)的 `OPEN ON`、`CLOSE ON` 和 `CLEAR` 子句会向权益账户记账。下面这些 Beancount 选项为这些账户命名，默认值与 Beancount 相同：

| 键 | 它命名的账户 | 默认值 |
|---|---|---|
| `account_previous_balances` | `OPEN ON` 的期初余额 | `Opening-Balances` |
| `account_previous_earnings` | `OPEN ON` 的以前收益 | `Earnings:Previous` |
| `account_previous_conversions` | `OPEN ON` 的以前转换 | `Conversions:Previous` |
| `account_current_earnings` | `CLEAR` 的收益 | `Earnings:Current` |
| `account_current_conversions` | `CLOSE ON` 的转换 | `Conversions:Current` |
| `conversion_currency` | 转换记账行的零价格所用的货币 | `NOTHING` |

账户选项给出的是名字中 `Equity:` 之后的部分。不是有效账户名的值会被忽略。见[权益账户](/zh-cn/reference/query-language/#权益账户)。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`MultipleOperatingCurrencyDetect`](/zh-cn/reference/error-codes/#multipleoperatingcurrencydetect) | `operating_currency` 被设置了不止一次。 |
| [`UnsupportedBookingMethod`](/zh-cn/reference/error-codes/#unsupportedbookingmethod) | `default_booking_method` 为 `AVERAGE`、`AVERAGE_ONLY` 或 `NONE`。 |
| [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta) | `default_booking_method` 不是一种记账方法。 |

无效的 `default_rounding` 或 `directive_output_path` 不会作为账本错误报告：账本直接无法加载。

## Beancount 兼容性

- 张记账读取 Beancount 的 `title` 和 `operating_currency` 选项。Beancount 允许设置多种主货币，Fava 会把它们并排显示；张记账只支持一种，并报告其余的。
- Beancount 自己的记账方法选项是 `booking_method`，默认值为 `STRICT`。张记账不读取 `booking_method`：请设置 `default_booking_method`，它的默认值为 `FIFO`。
- 查询会读取 Beancount 的 `account_previous_*`、`account_current_*` 和 `conversion_currency` 选项，见上文。
- 其他所有 Beancount 选项，例如 `inferred_tolerance_default`、`documents`、`render_commas` 或 `name_assets`，都会被保留，但没有作用。无论 `name_*` 选项怎样设置，账户名都必须以 `Assets`、`Liabilities`、`Equity`、`Income` 或 `Expenses` 开头。
- `default_*`、`timezone`、`directive_output_path` 和 `features.*` 是张记账自己的选项。Beancount 会把它们报告为无效选项，所以设置了它们的文件无法通过 `bean-check`。

## 相关页面

- [记录交易](/zh-cn/guides/recording-transactions/)：网页界面把新条目写到哪里。
- [商品](/zh-cn/reference/directives/commodity/)：每种商品的精度和舍入方式。
