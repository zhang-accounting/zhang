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

## 行为

- **加载账本时不会检查保存的查询。** 无法编译的查询（例如为 Zhang 尚未支持的功能编写的查询）依然会被保存，也不会让账本报告错误。
- **每一条 `query` 指令都会被保留，包括同名的查询。** 查询按账本顺序列出：先按日期，再按它们在文件中出现的顺序。同名的查询可以通过日期区分。

## HTTP API

`GET /api/query/saved` 列出保存的查询，每一项包含 `name`、`query`、`date`（`YYYY-MM-DD`）、`valid` 和 `error`。`valid` 表示该查询能否被当前的查询引擎编译；不能编译时，`error` 给出原因，并在可以定位时附带行号和列号。要执行保存的查询，把它的 `query` 文本发送到 `POST /api/query`。
