---
title: 自定义
description: custom 指令的参考，它为插件和工具携带带日期的值。
sidebar:
  order: 9
---

`custom` 指令保存张记账本身不解释的带日期的值。插件和其他工具会读取它们，例如用来获取一项随时间变化的设置。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] custom "<Type>" <Value> [<Value> …]
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 这些值从哪一天起适用，可以附带一天中的时刻。 |
| `"<Type>"` | 是 | 这条指令是关于什么的。按照惯例，写读取它的插件的名称。 |
| `<Value>` | 至少一个 | 用空格分隔的值。每个值是一个带引号的字符串、一个账户名，或者一个不含空格、引号、冒号、括号或逗号的单词，例如 `100`、`CNY`、`2024-07-01` 或 `TRUE`。 |

指令下方可以写元数据行。

## 示例

```zhang
2024-01-01 custom "large-expense" "threshold" 100 CNY
2024-07-01 custom "large-expense" "threshold" "150 CNY"
2024-01-01 custom "reconcile" Assets:Bank:Checking "monthly"
```

## 行为

- 张记账保存 `custom` 指令，但不检查其中的任何内容：值中提到的账户不必存在。
- 所有值都是文本。`100 CNY` 是 `"100"` 和 `"CNY"` 两个值；把它们当作金额来读取，由读取方自己决定。
- 网页界面不显示 `custom` 指令。在查询中，它们是 `#entries` 中类型为 `custom` 的行；插件会与账本的其他内容一起收到它们。

### 随时间变化的插件设置

用 Rust SDK 构建的插件，从以下形式的 `custom` 指令中读取设置：

```text
YYYY-MM-DD custom "<plugin name>" "<key>" <Value> …
```

其中 `<plugin name>` 是插件报告的名称。设置从它的日期起生效：按照上面的示例，日期为 2024-03-05 的条目看到的阈值是 `100 CNY`，日期为 2024-08-01 的条目看到的是 `150 CNY`。同一天同一个键有多条指令时，以最后一条为准。条目自身元数据中的设置优先于 `custom` 指令，而 `custom` 指令又优先于 `plugin` 指令的元数据和选项。只有 processor 插件能看到整个账本，所以只有 processor 会读取 `custom` 设置。见 [`custom` 指令中的配置](/zh-cn/developers/writing-plugins/#custom-指令中的配置)。

## 错误

`custom` 指令不会产生错误。读取它的插件可能会报告 [`PluginError`](/zh-cn/reference/error-codes/#pluginerror)。

## Beancount 兼容性

- Beancount 要求类型是带引号的字符串，值必须是带引号的字符串、数字、金额、日期、布尔值或账户。单独的裸词（例如 `CNY`）或不加引号的 `monthly`，在 Beancount 中是语法错误。张记账两种写法都能读取；如果文件还要在 Beancount 或 Fava 中加载，请按 Beancount 的写法书写值。
- 在 Beancount 文件中，张记账把 Beancount 接受的写法 `custom "budget" "Food" "CNY"`、`custom "budget-add" "Food" 2000 CNY`、`custom "budget-transfer" "Fun" "Food" 100 CNY` 和 `custom "budget-close" "Food"` 读作[预算指令](/zh-cn/reference/directives/budget/#beancount-兼容性)，也以这种方式写入预算。早期版本写出的不加引号的 `custom budget Food CNY` 仍然可以读取。带有其他值的 `custom "budget"`，例如 Fava 的 `custom "budget" Expenses:Coffee "daily" 4.00 EUR`，仍然是 `custom` 指令，其他任何类型也一样。

## 相关页面

- [编写插件](/zh-cn/developers/writing-plugins/)：在插件中读取 `custom` 指令。
- [插件](/zh-cn/reference/directives/plugin/)：声明插件。
