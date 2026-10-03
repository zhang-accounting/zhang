---
title: 备注与事件
description: note 和 event 指令的参考，它们为账户附加带日期的备注，或记录某个具名事件的取值。
sidebar:
  order: 8
---

`note` 为账户附加一条带日期的备注，例如“就手续费给银行打了电话”。`event` 记录你生活中某个具名变量在某一天取了新的值，例如你所在的城市或雇主。两者都不改变余额。你可以用查询把它们读出来。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] note <Account> "<Text>" [#tag …] [^link …]
YYYY-MM-DD [HH:MM[:SS]] event "<Type>" "<Value>"
```

| 指令 | 部分 | 必填 | 说明 |
|---|---|---|---|
| `note` | `<Account>` | 是 | 备注涉及的账户。 |
| | `"<Text>"` | 是 | 备注内容。 |
| | `#tag`、`^link` | 否 | 标签和链接，与交易上的写法相同。 |
| `event` | `"<Type>"` | 是 | 变量的名称，例如 `location`。 |
| | `"<Value>"` | 是 | 它的新值，例如一个城市。 |

两者都需要日期，可以附带一天中的时刻，下方可以写元数据行。字符串如果是一个不含空格、引号、冒号、括号或逗号的单词，张记账也能读取不加引号的写法。Beancount 要求加引号。

## 示例

```zhang
2024-01-01 open Assets:Bank:Checking CNY

2024-03-02 note Assets:Bank:Checking "Called the bank about the card fee" #fees ^case-1024
  contact: "support desk"

2024-04-01 event "location" "Tokyo"
2024-09-15 event "location" "Shanghai"
2024-05-01 event "employer" "ACME Ltd"
```

## 行为

- `note` 需要它的账户在其日期已经开立。与 Beancount 一样，备注可以写在账户的 `close` 之后。
- `event` 不会与任何东西核对。
- 网页界面的页面不显示备注和事件；你可以在文件中和查询中看到它们。插件会与账本的其他内容一起收到它们。
- 在查询中，[`#notes`](/zh-cn/reference/query-language/#pricesbalancesnoteseventsdocuments-和-commodities) 中每条备注对应一行，`#events` 中每个事件对应一行，列为 `date, type, description`。两者也都是 `#entries` 中的行。

```sql
SELECT date, description FROM #events WHERE type = 'location' ORDER BY date
```

## 错误

| 错误 | 触发条件 |
|---|---|
| [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist) | `note` 的账户从未开立，或者开立日期更晚。 |

`event` 不会产生错误。

## Beancount 兼容性

两个指令在 Beancount 中的语法都相同。在 Beancount 文件中，一天中的时刻写成 `time: "HH:MM:SS"` 元数据。

## 相关页面

- [查询](/zh-cn/guides/querying/)：把备注和事件读出来。
- [自定义](/zh-cn/reference/directives/custom/)：供插件和工具使用的带日期的值。
