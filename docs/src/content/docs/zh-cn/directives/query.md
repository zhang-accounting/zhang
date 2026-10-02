---
title: 查询
description: 使用 query 指令在账本中保存命名查询。
---

# 查询指令

`query` 指令在账本中保存一个命名查询，用法与 Beancount 和 Fava 相同。Zhang 会在查询页面的 **已保存** 菜单中列出保存的查询，方便再次执行而无需重新输入（见[保存的查询](/zh-cn/user-guide/query-language/#保存的查询)）。查询本身使用 Zhang 的[查询语言](/zh-cn/user-guide/query-language/)编写。

## 基本语法

```zhang
{DATE} query "{NAME}" "{QUERY}"
```

```zhang
2024-01-01 query "food by payee" "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee"
```

- 查询文本必须用引号包裹，可以跨越多行。
- 与其他带日期的指令一样，指令下方可以写元数据行。
- Beancount 文件（`.bean`）使用相同的语法，因此为 Fava 编写的账本可以保留其中保存的查询。

## 转义

查询文本与账本文件中的其他字符串一样，是带引号的字符串，所以其中的反斜杠表示转义序列的开始：

- 每个反斜杠都要写两次。要保存正则表达式 `\d+`，请写 `'\\d+'`：

  ```zhang
  2024-01-01 query "numbered" "SELECT narration WHERE narration ~ '\\d+'"
  ```

- 双引号写作 `\"`，或者像本页的示例那样，查询内部的字符串使用单引号。

反斜杠后面跟着不是已知转义的字符（例如只写了一个反斜杠的 `'\d+'`）会报错，导致账本无法加载（[#442](https://github.com/zhang-accounting/zhang/issues/442)）。Beancount 对写两次的形式有相同的解读，所以这样写的账本在两者中都可以使用。

## 行为

- **加载账本时不会检查保存的查询。** 无法编译的查询（例如为 Zhang 尚未支持的功能编写的查询）依然会被保存，也不会让账本报告错误。
- **每一条 `query` 指令都会被保留，包括同名的查询。** 查询按账本顺序列出：先按日期，再按它们在文件中出现的顺序。同名的查询可以通过日期区分。

## HTTP API

`GET /api/query/saved` 列出保存的查询，每一项包含 `name`、`query`、`date`（`YYYY-MM-DD`）、`valid` 和 `error`。`valid` 表示该查询能否被当前的查询引擎编译；不能编译时，`error` 给出原因，并在可以定位时附带行号和列号。要执行保存的查询，把它的 `query` 文本发送到 `POST /api/query`。
