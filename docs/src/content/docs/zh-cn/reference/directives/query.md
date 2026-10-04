---
title: 查询
description: query 指令的参考，它在账本中保存一条具名查询。
sidebar:
  order: 13
---

`query` 指令在账本中保存一条具名查询，用法与 Beancount 和 Fava 相同。张记账在查询页面的 **已保存** 菜单中列出保存的查询，方便你再次执行而无需重新输入（见[保存的查询](/zh-cn/reference/query-language/#保存的查询)）。查询本身用张记账的[查询语言](/zh-cn/reference/query-language/)编写。

## 语法

```text
YYYY-MM-DD [HH:MM[:SS]] query "<Name>" "<Query>"
```

| 部分 | 必填 | 说明 |
|---|---|---|
| 日期和时间 | 是 | 查询的日期。它决定列表的顺序，并用来区分同名的查询。 |
| `"<Name>"` | 是 | 在 **已保存** 菜单中显示的名称。单个单词可以不加引号。 |
| `"<Query>"` | 是 | 查询文本，写在双引号中。可以跨越多行。 |

指令下方可以写元数据行。

## 示例

```zhang
2024-01-01 query "food by payee" "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee"

2024-02-01 query "monthly food" "
  SELECT year, month, sum(position)
  WHERE account ~ '^Expenses:Food'
  GROUP BY year, month"
  owner: "alice"
```

## 转义

查询文本与账本文件中的其他字符串一样，是带引号的字符串，所以其中的反斜杠可以开始一个转义序列：

- `\"` 表示双引号，`\\` 表示反斜杠。要在查询中使用双引号，请写 `\"`，或者像本页的示例那样，查询内部的字符串使用单引号。
- `\n`、`\t`、`\r`、`\b`、`\f`、`\/` 和 `\uXXXX` 的含义与 JSON 中相同。旧版本张记账写出的转义仍会被读作它们所代表的字符：`\$`、`` \` ``、`\a`、`\v`、`\e` 和 `\u{...}`（例如 `\u{a0}`）。
- 反斜杠后面跟着其他字符时，会原样保留。所以正则表达式 `\d+` 可以直接写：

  ```zhang
  2024-01-01 query "numbered" "SELECT narration WHERE narration ~ '\d+'"
  ```

- 把反斜杠写两次，例如 `'\\d+'`，同样可以使用，读取结果相同。当反斜杠后面是上面列出的字符时，请写两次。在正则表达式中，`'\\b'` 表示单词边界，而 `'\b'` 会被读作退格符；`'\\$'` 匹配美元符号，而 `'\$'` 会被读作 `$`，即文本的结尾。

格式错误的 `\u` 转义（例如 `\uZZZZ`）是一个错误，会让账本无法加载。错误信息会给出该转义所在的行和列。

## 行为

- **加载账本时不会检查保存的查询**。无法编译的查询（例如为张记账尚未支持的功能编写的查询）仍会被保存，也从不会让账本报告错误。**已保存** 菜单和 HTTP API 会说明每条查询能否编译。
- **每一条 `query` 指令都会被保留，包括同名的查询**。查询按账本顺序列出：先按日期，再按它们在文件中出现的顺序。

## HTTP API

`GET /api/query/saved` 列出保存的查询：

```json
{
  "data": [
    {
      "name": "food by payee",
      "query": "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee",
      "date": "2024-01-01",
      "valid": true,
      "error": null
    }
  ]
}
```

`valid` 表示该查询能否被当前的查询引擎编译。不能编译时，`error` 给出原因，并在已知时附带行号和列号。要执行保存的查询，把它的 `query` 文本发送到 `POST /api/query`。

## 错误

`query` 指令不会产生账本错误。

## Beancount 兼容性

Beancount 文件（`.bean`）使用相同的语法，所以为 Fava 编写的账本可以保留其中保存的查询。张记账按上面的转义规则读取这些字符串。Beancount 本身会丢弃不构成已知转义的反斜杠，所以它把 `'\d+'` 读作 `'d+'`。它也没有 `\uXXXX`、`\a`、`\v` 或 `\e` 转义，所以它把 `\u00e9` 读作 `u00e9`，而张记账读作 `é`。如果账本也要在 Beancount 或 Fava 中使用，请把每个反斜杠写两次，例如 `'\\d+'`。这种写法在两者中的读取结果相同。

张记账的查询语言与 Beancount 的有哪些不同，见[与 BQL 和 beanquery 的差异](/zh-cn/reference/query-language/#与-bql-和-beanquery-的差异)。

## 相关页面

- [查询](/zh-cn/guides/querying/)：查询页面和常用查询。
- [查询语言](/zh-cn/reference/query-language/)：完整的参考。
