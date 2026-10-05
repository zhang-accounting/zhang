---
title: 账户
description: open 和 close 指令的参考，它们开始和结束一个账户的使用。
sidebar:
  order: 2
---

`open` 指令创建一个账户，并让它从指令的日期起可以使用。`close` 指令结束它的使用。交易记账的每个账户都必须先开立。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] open <Account> [<Commodity>[, <Commodity> …]]
  [<key>: <value>]
YYYY-MM-DD [HH:MM[:SS]] close <Account>
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 账户开立或关闭的时间，可以附带一天中的时刻。 |
| `<Account>` | 是 | 账户名，例如 `Assets:Bank:Checking`。 |
| `<Commodity>, …` | 否 | 这个账户只能持有的商品，用逗号分隔。每种商品都必须已经定义。不写则允许任何商品。 |
| `<key>: <value>` | 否 | 元数据行。张记账读取的键见[元数据](#元数据)。 |

**账户名**以五种账户类型之一开头：`Assets`、`Liabilities`、`Equity`、`Income` 或 `Expenses`，后面跟一个或多个用 `:` 分隔的部分。每个部分可以是任何不含空格、引号、冒号、括号或逗号的文本，所以 `Expenses:Food:餐饮` 是有效的账户名。

## 示例

```zhang
2024-01-01 commodity USD
2024-01-01 budget Food CNY

2024-01-01 open Assets:Bank:Checking CNY
  alias: "Main checking"
2024-01-01 open Assets:Broker USD
  booking_method: "STRICT"
2024-01-01 open Expenses:Food CNY
  budget: Food
2024-01-01 open Equity:Opening-Balances

2024-12-31 close Assets:Broker
```

## 元数据

| 键 | 作用 |
|---|---|
| `alias` | 账户的显示名称。账户列表显示这个名称，并在它下方显示账户的全名。 |
| `booking_method` | 账户的记账方法，见[记账方法](#记账方法)。 |
| `budget` | 把账户关联到一个[预算](/zh-cn/reference/directives/budget/#关联账户)：它的记账行计入该预算的支出。重复这个键可以关联多个预算。 |

其他元数据会被保留。在查询中，`#accounts` 以 `open.meta` 给出 `open` 指令的元数据。

### 记账方法

记账方法决定减仓（例如 `-5 AAPL {}`）从哪个批次中扣除数量。没有这项元数据时，账户使用 [`default_booking_method`](/zh-cn/reference/directives/options/#default_booking_method) 选项（未设置时为 `FIFO`）。

与 Beancount 相同，减仓匹配其成本所指定的批次，成本中没有写出的部分可以匹配任何值：`{100 USD}` 匹配所有以 100 USD 持有的批次，不论取得日期；`{100 USD, 2024-01-01}` 只匹配在该日取得的批次；`{}` 匹配所有按成本持有的批次。批次的取得日期是其成本中写明的日期，否则是建立该批次的交易的日期。

- `STRICT`：Beancount 的默认方法。减仓必须只匹配一个批次，或者把匹配到的每个批次全部减完。否则账本报告 [`AmbiguousLotMatch`](/zh-cn/reference/error-codes/#ambiguouslotmatch) 错误，并在匹配的批次中按 `FIFO` 记账。
- `FIFO`：先进先出，先减取得日期最早的匹配批次。
- `LIFO`：后进先出，先减取得日期最晚的匹配批次。

`AVERAGE`、`AVERAGE_ONLY` 和 `NONE` 尚未实现。使用其中之一的账户，或者值不是记账方法的账户，会在其 `open` 指令上得到一个错误（[`UnsupportedBookingMethod`](/zh-cn/reference/error-codes/#unsupportedbookingmethod) 或 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta)），并按默认的记账方法记账。账本仍然可以加载。批次的用法见[批次与成本](/zh-cn/guides/lots-and-cost-basis/)。

## 行为

### 账户何时可用

账户是否可用由同一条规则决定，所有使用账户的指令和网页界面都遵循它：

- 账户从 `open` 起可用。在同一日期和时间内，`open` 排在其他所有指令之前，所以 `open` 当天的交易没有问题。
- 账户在 `close` 生效之前一直可用。只有日期的 `close` 在当天结束（24:00）时关闭账户，所以当天的所有指令都还可以使用它。在 zhang 文件中，带时间的 `close` 在该时刻关闭账户：正好在该时刻的指令还可以使用它，当天更晚的指令则不行。在 Beancount 文件中，与 Beancount 一样，`close` 的 `time` 元数据只是普通元数据：账户在当天全天仍可使用。
- 在 `close` 之后再写 `open` 会重新开立账户。
- 指令使用了从未开立、或者开立时间更晚的账户，会报告 [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist)。
- 在账户关闭之后记账，会报告 [`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed)：记账行、[`pad` 或 `balance … with pad`](/zh-cn/reference/directives/balance/)。
- 只做记录的指令可以出现在 `close` 之后，与 Beancount 一样：普通的 `balance`（例如断言已关闭的账户余额为零）、[`document`](/zh-cn/reference/directives/document/)（例如最后一期对账单）和 [`note`](/zh-cn/reference/directives/note-and-event/)。
- 每个错误对每个账户和指令各报告一次，指令仍然有效：交易仍会记账，文档仍会列出。
- 开立一个账户不会开立它的父账户。不开立 `Assets:Bank` 也可以使用 `Assets:Bank:Checking`，但对 `Assets:Bank` 的[余额断言](/zh-cn/reference/directives/balance/)需要 `Assets:Bank` 已经开立。

```zhang
2024-01-01 open Assets:Wallet
2024-03-31 close Assets:Wallet
2024-03-31 18:00 * "Last coffee"     ; 没有问题：账户在 3 月 31 日当天仍可用
  Assets:Wallet -3 CNY
  Expenses:Coffee
2024-04-01 * "Too late"              ; AccountClosed
  Assets:Wallet -3 CNY
  Expenses:Coffee
2024-04-02 balance Assets:Wallet -6 CNY  ; 没有问题：余额断言只做记录
```

账户列表按账本时钟，从账户的 `close` 生效的那一刻起把它显示为已关闭，在之后的 `open` 之后又显示为开立。

### 商品

`open` 中列出的商品必须在它之前[定义](/zh-cn/reference/directives/commodity/)；未定义的商品会在 `open` 上报告 [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine)。同一日期内，把 `commodity` 指令写在 `open` 上方。查询在 `#accounts` 中以 `open.currencies` 读取这个列表。

与 Beancount 一样，列表把账户限定在它列出的商品之内：商品不同的记账行、[余额断言](/zh-cn/reference/directives/balance/)或补齐，每个写下的记账行各报告一次 [`CommodityNotAllowed`](/zh-cn/reference/error-codes/#commoditynotallowed)，带有 `account_name` 和 `commodity` 元数据。账本仍会加载，交易仍会记账。

- 只检查记账行的数量，不检查它的成本和价格，所以以 `AAPL` 开立的账户可以买入 `AAPL {90 EUR}`。
- 没有列表的 `open` 允许任何商品。限制只针对账户本身，子账户不受限制。
- 被重新开立的账户，以该指令之前最近一次 `open` 的列表为准。

### 关闭

- `close` 检查账户自身在每种商品上的余额，不含子账户。余额不为零时报告 [`CloseNonZeroAccount`](/zh-cn/reference/error-codes/#closenonzeroaccount)。账户仍会被关闭。
- 关闭从未开立的账户会报告 `AccountDoesNotExist`，关闭已关闭的账户会报告 `AccountClosed`，以第一次 `close` 为准。
- 已关闭的账户保留它的余额和历史。账户列表把它标记为已关闭，并且可以隐藏它。
- `close` 之后可以有普通的 `balance`、[`document`](/zh-cn/reference/directives/document/) 和 [`note`](/zh-cn/reference/directives/note-and-event/)，不会报错。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`CommodityDoesNotDefine`](/zh-cn/reference/error-codes/#commoditydoesnotdefine) | `open` 中列出的商品未定义。 |
| [`CommodityNotAllowed`](/zh-cn/reference/error-codes/#commoditynotallowed) | 记账行、余额断言或补齐使用了账户的 `open` 没有列出的商品。 |
| [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta) | `booking_method` 不是一种记账方法。 |
| [`UnsupportedBookingMethod`](/zh-cn/reference/error-codes/#unsupportedbookingmethod) | `booking_method` 为 `AVERAGE`、`AVERAGE_ONLY` 或 `NONE`。 |
| [`CloseNonZeroAccount`](/zh-cn/reference/error-codes/#closenonzeroaccount) | 账户关闭时仍持有某种商品。 |
| [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist) | 指令使用了尚未开立的账户，或者关闭了从未开立的账户。 |
| [`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed) | 记账行、`pad` 或 `balance … with pad` 在账户关闭之后使用了它，或者已关闭的账户再次被关闭。 |

## Beancount 兼容性

- Beancount 把记账方法写成商品之后的带引号字符串。张记账在 Beancount 文件中读取这种写法，并把它存为 `booking_method` 元数据。在 zhang 文件中请改用元数据：商品后面的字符串是语法错误。

  ```beancount
  2024-01-01 open Assets:Broker USD "FIFO"
  ```

- 两者都会报告商品不在 `open` 列表中的记账行，以及这种商品的余额断言。对按多个批次记账的卖出，Beancount 每个批次报告一次，张记账对写下的记账行报告一次。张记账允许重新开立账户，以最近一次 `open` 的列表为准；Beancount 会把第二次 `open` 报告为错误。
- Beancount 的默认记账方法是 `STRICT`；张记账的是 `FIFO`。
- `CloseNonZeroAccount` 是张记账自己的检查：Beancount 关闭这样的账户时不会报错。
- 两者都让账户在 `close` 当天保持可用：Beancount 把 `close` 排在当天所有其他指令之后。Beancount 没有时间的概念；在 Beancount 文件中，张记账同样把 `close` 的 `time` 元数据当作普通元数据。
- 两者都接受 `close` 之后的 `balance`、`document` 和 `note`，并报告记账行或 `pad`。

## 相关页面

- [余额](/zh-cn/reference/directives/balance/)：余额断言和补齐。
- [批次与成本](/zh-cn/guides/lots-and-cost-basis/)：记账方法的实际运用。
- [预算](/zh-cn/reference/directives/budget/)：把支出账户关联到预算。
